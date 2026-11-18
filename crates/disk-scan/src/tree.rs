//! Compact native names and adjacency. Full paths are reconstructed from checked topology.
use crate::{Identity, Node, NodeKind, Snapshot, MAX_NODES};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::{
    borrow::Cow,
    ffi::OsStr,
    path::{Path, PathBuf},
};
mod records;
use records::Records;
const NO_PARENT: u32 = u32::MAX;
#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub device: u64,
    pub inode: u64,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub own_logical_size: u64,
    pub own_allocated_size: u64,
    pub name_offset: u64,
    pub children_offset: u64,
    pub parent: u32,
    pub name_len: u32,
    pub children_len: u32,
    pub flags: u32,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    pub records: Records,
    pub names: Vec<u8>,
    pub children: Vec<u32>,
    pub paths: Vec<(u32, PathBuf)>,
    pub labels: Vec<(u32, String)>,
    pub clones: Vec<(u32, crate::CloneMetadata)>,
    pub clone_attributions: Vec<(u32, (bool, u64))>,
}
#[derive(Clone, Debug)]
pub struct NodeRef<'a> {
    snapshot: &'a Snapshot,
    pub id: u32,
    pub parent: Option<u32>,
    pub identity: Identity,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub own_logical_size: u64,
    pub own_allocated_size: u64,
    pub is_directory: bool,
    pub hard_link: bool,
    pub excluded: bool,
}
impl<'a> NodeRef<'a> {
    pub fn clone_metadata(&self) -> Option<&'a crate::CloneMetadata> {
        lookup(&self.snapshot.tree.clones, self.id)
    }
    pub fn clone_attribution(&self) -> Option<(bool, u64)> {
        self.snapshot.clone_attribution(self.id)
    }
    pub fn path(&self) -> Result<PathBuf, String> {
        self.snapshot.path(self.id)
    }
    pub fn native_name(&self) -> &'a OsStr {
        self.snapshot
            .native_name(self.id)
            .expect("checked compact name")
    }
    pub fn display_name(&self) -> Cow<'a, str> {
        self.snapshot
            .display_name(self.id)
            .expect("checked compact label")
    }
    pub fn children(&self) -> &'a [u32] {
        self.snapshot
            .children(self.id)
            .expect("checked compact adjacency")
    }
}
fn lookup<T>(entries: &[(u32, T)], id: u32) -> Option<&T> {
    entries
        .binary_search_by_key(&id, |e| e.0)
        .ok()
        .map(|i| &entries[i].1)
}
/// Owning streaming decoder with two bounded parent-prefix buffers. The cache can
/// only refer to immutable prior records of this builder's snapshot, never caller
/// supplied traversal authority. Finishing drops all decoder scratch.
pub struct LegacyTreeBuilder {
    snapshot: Snapshot,
    parent_id: Option<u32>,
    parent_path: PathBuf,
    expected_path: PathBuf,
}
impl LegacyTreeBuilder {
    pub fn new(root: PathBuf) -> Self {
        Self {
            snapshot: Snapshot::empty(root),
            parent_id: None,
            parent_path: PathBuf::new(),
            expected_path: PathBuf::new(),
        }
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn node_count(&self) -> usize {
        self.snapshot.node_count()
    }
    #[cfg(test)]
    pub fn scratch_capacity_bytes(&self) -> usize {
        self.parent_path.capacity() + self.expected_path.capacity()
    }
    pub fn append(&mut self, n: Node) -> Result<(), String> {
        self.append_ref(&n)
    }
    /// Copies compact bytes before returning; ordinary paths and labels are not cloned.
    pub fn append_ref(&mut self, n: &Node) -> Result<(), String> {
        self.snapshot.check_append(n)?;
        if n.id == 0 && self.snapshot.root.as_os_str().is_empty() {
            self.snapshot.root = n.path.clone();
        }
        if let Some(parent) = n.parent {
            if self.parent_id != Some(parent) {
                self.snapshot.path_into(parent, &mut self.parent_path)?;
                self.parent_id = Some(parent);
            }
            self.expected_path.clone_from(&self.parent_path);
            self.expected_path
                .push(n.path.file_name().unwrap_or(n.path.as_os_str()));
        } else {
            self.expected_path.clone_from(&self.snapshot.root);
        }
        self.snapshot
            .append_legacy_ref(n, Some(&self.expected_path))
    }
    pub fn finish(self) -> Snapshot {
        self.snapshot
    }
}
/// Bounded path scratch for immutable scans. At most one parent and one output path
/// are retained, irrespective of tree width; exact legacy overrides stay authoritative.
pub struct NodePathCursor<'a> {
    snapshot: &'a Snapshot,
    parent_id: Option<u32>,
    parent_path: PathBuf,
    output: PathBuf,
}
impl NodePathCursor<'_> {
    /// Requested bytes of the two bounded owned path buffers (excluding headers).
    pub fn capacity_bytes(&self) -> usize {
        self.parent_path.capacity() + self.output.capacity()
    }
    pub fn path(&mut self, id: u32) -> Result<&Path, String> {
        self.paths(id).map(|(path, _)| path)
    }
    pub fn paths(&mut self, id: u32) -> Result<(&Path, Option<&Path>), String> {
        let node = self.snapshot.node(id).ok_or("invalid node id")?;
        let parent = if id == 0 {
            if node.parent.is_some() {
                return Err("root has parent".into());
            }
            None
        } else {
            let parent = node
                .parent
                .filter(|parent| *parent < id)
                .ok_or("invalid path ancestry")?;
            if self.parent_id != Some(parent) {
                self.snapshot.path_into(parent, &mut self.parent_path)?;
                self.parent_id = Some(parent);
            }
            Some(self.parent_path.as_path())
        };
        if id == 0 || lookup(&self.snapshot.tree.paths, id).is_some() {
            self.snapshot.path_into(id, &mut self.output)?;
        } else {
            let name = self.snapshot.native_name(id)?;
            #[cfg(unix)]
            let ordinary = is_exact_native_component(name);
            #[cfg(not(unix))]
            let ordinary = {
                let mut components = Path::new(name).components();
                matches!(components.next(), Some(std::path::Component::Normal(_)))
                    && components.next().is_none()
            };
            if ordinary {
                #[cfg(unix)]
                join_native_component_into(&self.parent_path, name, &mut self.output)?;
                #[cfg(not(unix))]
                {
                    self.output.clone_from(&self.parent_path);
                    self.output
                        .as_os_str()
                        .len()
                        .checked_add(name.len())
                        .and_then(|n| n.checked_add(1))
                        .ok_or("path length overflow")?;
                    self.output.push(name);
                }
            } else {
                self.snapshot.path_into(id, &mut self.output)?;
            }
        }
        Ok((self.output.as_path(), parent))
    }
}
fn is_exact_native_component(name: &OsStr) -> bool {
    #[cfg(unix)]
    {
        let bytes = name.as_bytes();
        !bytes.is_empty() && bytes != b"." && bytes != b".." && !bytes.contains(&b'/')
    }
    #[cfg(not(unix))]
    {
        let mut components = Path::new(name).components();
        matches!(components.next(),Some(std::path::Component::Normal(component)) if component==name)
            && components.next().is_none()
    }
}
#[cfg(unix)]
fn join_native_component_into(
    parent: &Path,
    name: &OsStr,
    output: &mut PathBuf,
) -> Result<(), String> {
    use std::os::unix::ffi::OsStringExt;
    let prefix = parent.as_os_str().as_bytes();
    let name = name.as_bytes();
    let separator = usize::from(!prefix.is_empty() && prefix.last() != Some(&b'/'));
    let total = prefix
        .len()
        .checked_add(separator)
        .and_then(|n| n.checked_add(name.len()))
        .ok_or("path length overflow")?;
    let mut bytes = std::mem::take(output).into_os_string().into_vec();
    bytes.clear();
    bytes.reserve(total);
    bytes.extend_from_slice(prefix);
    if separator != 0 {
        bytes.push(b'/');
    }
    bytes.extend_from_slice(name);
    *output = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    Ok(())
}
impl Snapshot {
    pub fn path_cursor(&self) -> NodePathCursor<'_> {
        NodePathCursor {
            snapshot: self,
            parent_id: None,
            parent_path: PathBuf::new(),
            output: PathBuf::new(),
        }
    }
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
    pub fn node_count(&self) -> usize {
        self.tree.records.len()
    }
    pub fn node(&self, id: u32) -> Option<NodeRef<'_>> {
        let r = self.tree.records.get(id as usize)?;
        let kind = match r.flags & 3 {
            0 => NodeKind::Directory,
            1 => NodeKind::File,
            2 => NodeKind::Symlink,
            _ => NodeKind::Other,
        };
        Some(NodeRef {
            snapshot: self,
            id,
            parent: (r.parent != NO_PARENT).then_some(r.parent),
            identity: Identity {
                device: r.device,
                inode: r.inode,
                kind,
            },
            logical_size: r.logical_size,
            allocated_size: r.allocated_size,
            own_logical_size: r.own_logical_size,
            own_allocated_size: r.own_allocated_size,
            is_directory: r.flags & 4 != 0,
            hard_link: r.flags & 8 != 0,
            excluded: r.flags & 16 != 0,
        })
    }
    pub fn iter_nodes(&self) -> impl ExactSizeIterator<Item = NodeRef<'_>> + DoubleEndedIterator {
        (0..self.node_count()).map(|id| self.node(id as u32).expect("dense compact ids"))
    }
    pub fn native_name(&self, id: u32) -> Result<&OsStr, String> {
        let r = self
            .tree
            .records
            .get(id as usize)
            .ok_or("invalid node id")?;
        let start = usize::try_from(r.name_offset).map_err(|_| "name offset overflow")?;
        let end = start
            .checked_add(r.name_len as usize)
            .ok_or("name range overflow")?;
        let bytes = self
            .tree
            .names
            .get(start..end)
            .ok_or("invalid name range")?;
        #[cfg(unix)]
        {
            Ok(OsStr::from_bytes(bytes))
        }
        #[cfg(not(unix))]
        {
            Ok(OsStr::new(
                std::str::from_utf8(bytes).map_err(|_| "invalid native name")?,
            ))
        }
    }
    /// Checked exact native basename for representation-based topology proofs.
    /// NUL handling belongs to the consumer's existing policy, not this path proof.
    pub fn native_component(&self, id: u32) -> Result<&OsStr, String> {
        let name = self.native_name(id)?;
        if is_exact_native_component(name) {
            Ok(name)
        } else {
            Err("native name is not one normal component".into())
        }
    }
    pub fn display_name(&self, id: u32) -> Result<Cow<'_, str>, String> {
        if let Some(label) = lookup(&self.tree.labels, id) {
            return Ok(Cow::Borrowed(label));
        }
        Ok(self.native_name(id)?.to_string_lossy())
    }
    pub fn children(&self, id: u32) -> Result<&[u32], String> {
        let r = self
            .tree
            .records
            .get(id as usize)
            .ok_or("invalid node id")?;
        let start = usize::try_from(r.children_offset).map_err(|_| "child offset overflow")?;
        let end = start
            .checked_add(r.children_len as usize)
            .ok_or("child range overflow")?;
        self.tree
            .children
            .get(start..end)
            .ok_or_else(|| "invalid child range".into())
    }
    pub fn path_into(&self, id: u32, scratch: &mut PathBuf) -> Result<(), String> {
        if let Some(path) = lookup(&self.tree.paths, id) {
            scratch.clone_from(path);
            return Ok(());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let mut current = id;
            let mut suffix_len = 0usize;
            let mut depth = 0usize;
            let anchor = loop {
                let r = self
                    .tree
                    .records
                    .get(current as usize)
                    .ok_or("invalid path node")?;
                if let Some(path) = lookup(&self.tree.paths, current) {
                    break path.as_os_str().as_bytes();
                }
                if current == 0 {
                    if r.parent != NO_PARENT {
                        return Err("root has parent".into());
                    }
                    break self.root.as_os_str().as_bytes();
                }
                let name = self.native_name(current)?;
                suffix_len = suffix_len
                    .checked_add(name.len())
                    .and_then(|n| n.checked_add(1))
                    .ok_or("path length overflow")?;
                depth += 1;
                if depth > self.node_count() || r.parent >= current {
                    return Err("invalid path ancestry".into());
                }
                current = r.parent;
            };
            let anchor_id = current;
            let skip_separator = depth > 0 && (anchor.is_empty() || anchor.last() == Some(&b'/'));
            let total = anchor
                .len()
                .checked_add(suffix_len)
                .and_then(|n| n.checked_sub(usize::from(skip_separator)))
                .ok_or("path length overflow")?;
            let mut bytes = std::mem::take(scratch).into_os_string().into_vec();
            bytes.clear();
            bytes.resize(total, 0);
            bytes[..anchor.len()].copy_from_slice(anchor);
            let mut position = total;
            current = id;
            while current != anchor_id {
                let name = self.native_name(current)?.as_bytes();
                position -= name.len();
                bytes[position..position + name.len()].copy_from_slice(name);
                if position > anchor.len() {
                    position -= 1;
                    bytes[position] = b'/';
                }
                current = self.tree.records[current as usize].parent;
            }
            *scratch = PathBuf::from(std::ffi::OsString::from_vec(bytes));
        }
        #[cfg(not(unix))]
        {
            let mut chain = Vec::new();
            let mut current = id;
            while current != 0 {
                let r = self
                    .tree
                    .records
                    .get(current as usize)
                    .ok_or("invalid path node")?;
                if r.parent >= current {
                    return Err("invalid path ancestry".into());
                }
                chain.push(current);
                current = r.parent;
            }
            scratch.clone_from(&self.root);
            for child in chain.into_iter().rev() {
                scratch.push(self.native_name(child)?);
            }
        }
        Ok(())
    }
    pub fn path(&self, id: u32) -> Result<PathBuf, String> {
        let mut p = PathBuf::new();
        self.path_into(id, &mut p)?;
        Ok(p)
    }
    pub fn append_legacy_node(&mut self, n: Node) -> Result<(), String> {
        self.append_legacy_ref(&n, None)
    }
    pub(crate) fn append_scanned_node(&mut self, n: &crate::ScannedNode) -> Result<(), String> {
        let name = n.path.native_name_or_path();
        let record = self.compact_record(n.id, &n.metadata, name, 0)?;
        self.tree.records.push(record);
        self.tree.names.extend_from_slice(name.as_encoded_bytes());
        Ok(())
    }
    fn check_append(&self, n: &Node) -> Result<(), String> {
        self.check_metadata(n.id, n.parent, &n.identity, n.is_directory, n.hard_link)
    }
    fn check_metadata(
        &self,
        id: u32,
        parent: Option<u32>,
        identity: &Identity,
        is_directory: bool,
        hard_link: bool,
    ) -> Result<(), String> {
        if self.node_count() >= MAX_NODES || id as usize != self.node_count() {
            return Err("invalid dense node id".into());
        }
        if is_directory != (identity.kind == NodeKind::Directory)
            || (hard_link && identity.kind != NodeKind::File)
        {
            return Err("contradictory node flags".into());
        }
        if id == 0 && parent.is_some() || id != 0 && parent.is_none_or(|p| p >= id) {
            return Err("invalid parent topology".into());
        }
        Ok(())
    }
    fn compact_record(
        &self,
        id: u32,
        m: &NodeMetadata,
        name: &OsStr,
        children_len: usize,
    ) -> Result<Record, String> {
        self.check_metadata(id, m.parent, &m.identity, m.is_directory, m.hard_link)?;
        let name_len = u32::try_from(name.len()).map_err(|_| "native name too long")?;
        let children_len = u32::try_from(children_len).map_err(|_| "too many children")?;
        Ok(Record {
            device: m.identity.device,
            inode: m.identity.inode,
            logical_size: m.logical_size,
            allocated_size: m.allocated_size,
            own_logical_size: m.own_logical_size,
            own_allocated_size: m.own_allocated_size,
            name_offset: self.tree.names.len() as u64,
            children_offset: self.tree.children.len() as u64,
            parent: m.parent.unwrap_or(NO_PARENT),
            name_len,
            children_len,
            flags: m.flags(),
        })
    }
    fn legacy_metadata(n: &Node) -> NodeMetadata {
        NodeMetadata {
            parent: n.parent,
            identity: n.identity.clone(),
            logical_size: n.logical_size,
            allocated_size: n.allocated_size,
            own_logical_size: n.own_logical_size,
            own_allocated_size: n.own_allocated_size,
            is_directory: n.is_directory,
            hard_link: n.hard_link,
            excluded: n.excluded,
        }
    }
    fn append_legacy_ref(
        &mut self,
        n: &Node,
        provided_expected: Option<&Path>,
    ) -> Result<(), String> {
        let basename = n.path.file_name();
        let name = basename.unwrap_or(n.path.as_os_str());
        let m = Self::legacy_metadata(n);
        let record = self.compact_record(n.id, &m, name, n.children.len())?;
        let computed;
        let expected = if let Some(path) = provided_expected {
            path
        } else {
            computed = if let Some(parent) = n.parent {
                self.path(parent)?.join(name)
            } else {
                self.root.clone()
            };
            computed.as_path()
        };
        // CORE-R2: paths without a normal basename retain supplied bytes even
        // when joining the absolute fallback would otherwise compare equal.
        if (n.id != 0 && basename.is_none()) || expected.as_os_str() != n.path.as_os_str() {
            self.tree.paths.push((n.id, n.path.clone()));
        }
        if n.name != name.to_string_lossy() {
            self.tree.labels.push((n.id, n.name.clone()));
        }
        self.push_legacy(n, record, name);
        Ok(())
    }
    fn push_legacy(&mut self, n: &Node, record: Record, name: &OsStr) {
        if let Some(metadata) = &n.clone_metadata {
            self.tree.clones.push((n.id, metadata.clone()));
        }
        self.tree.records.push(record);
        self.tree.names.extend_from_slice(name.as_encoded_bytes());
        self.tree.children.extend_from_slice(&n.children);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_children(&mut self, id: u32, children: &[u32]) -> Result<(), String> {
        let r = self
            .tree
            .records
            .get_mut(id as usize)
            .ok_or("invalid node id")?;
        r.children_len = u32::try_from(children.len()).map_err(|_| "too many children")?;
        r.children_offset = self.tree.children.len() as u64;
        self.tree.children.extend_from_slice(children);
        Ok(())
    }
    /// Scanner publication holds the store exclusively and appends only this
    /// directory's children. Write their dense IDs without a second scratch list.
    pub(crate) fn set_scanned_children(
        &mut self,
        id: u32,
        first_child: usize,
    ) -> Result<(), String> {
        let end = self.node_count();
        if first_child > end || first_child <= id as usize {
            return Err("invalid scanned child range".into());
        }
        let children_len = u32::try_from(end - first_child).map_err(|_| "too many children")?;
        let first_child = u32::try_from(first_child).map_err(|_| "invalid scanned child range")?;
        let end = u32::try_from(end).map_err(|_| "invalid scanned child range")?;
        let r = self
            .tree
            .records
            .get_mut(id as usize)
            .ok_or("invalid node id")?;
        r.children_len = children_len;
        r.children_offset = self.tree.children.len() as u64;
        self.tree.children.extend(first_child..end);
        Ok(())
    }
    pub fn legacy_node(&self, id: u32) -> Result<Node, String> {
        let n = self.node(id).ok_or("invalid node id")?;
        Ok(Node {
            clone_metadata: n.clone_metadata().cloned(),
            id,
            parent: n.parent,
            children: n.children().to_vec(),
            name: n.display_name().into_owned(),
            path: n.path()?,
            identity: n.identity,
            logical_size: n.logical_size,
            allocated_size: n.allocated_size,
            own_logical_size: n.own_logical_size,
            own_allocated_size: n.own_allocated_size,
            is_directory: n.is_directory,
            hard_link: n.hard_link,
            excluded: n.excluded,
        })
    }
    /// Reset totals without decoding or rewriting unrelated compact metadata.
    pub(crate) fn reset_aggregate_sizes(&mut self, mut allocation: impl FnMut(u32, u64) -> u64) {
        for id in 0..self.node_count() {
            let r = &mut self.tree.records[id];
            let allocated = allocation(id as u32, r.own_allocated_size);
            r.logical_size = r.own_logical_size;
            r.allocated_size = allocated;
        }
    }
    /// Preserve reverse-ID aggregation, including historical malformed-parent behavior.
    pub(crate) fn aggregate_sizes_to_parents(&mut self) -> Result<(), String> {
        let mut remaining = self.node_count();
        while remaining != 0 {
            let id = remaining - 1;
            let r = &self.tree.records[id];
            let (parent, logical, allocated) = (r.parent, r.logical_size, r.allocated_size);
            remaining = id;
            if parent != NO_PARENT {
                let (mut logical, mut allocated) = (logical, allocated);
                // Adjacent siblings share an earlier parent, so no record in
                // this run can depend on another record's pending total.
                // Saturating addition is associative for these unsigned sizes.
                while remaining != 0 && (parent as usize) < remaining - 1 {
                    let previous = &self.tree.records[remaining - 1];
                    if previous.parent != parent {
                        break;
                    }
                    logical = logical.saturating_add(previous.logical_size);
                    allocated = allocated.saturating_add(previous.allocated_size);
                    remaining -= 1;
                }
                // Self/forward/invalid parents retain the original one-record
                // operation and error order, including partial updates on error.
                let parent = self
                    .tree
                    .records
                    .get_mut(parent as usize)
                    .ok_or("invalid node id")?;
                parent.logical_size = parent.logical_size.saturating_add(logical);
                parent.allocated_size = parent.allocated_size.saturating_add(allocated);
            }
        }
        Ok(())
    }
    pub fn update_node(
        &mut self,
        id: u32,
        update: impl FnOnce(&mut NodeMetadata),
    ) -> Result<(), String> {
        let n = self.node(id).ok_or("invalid node id")?;
        let mut value = NodeMetadata {
            parent: n.parent,
            identity: n.identity,
            logical_size: n.logical_size,
            allocated_size: n.allocated_size,
            own_logical_size: n.own_logical_size,
            own_allocated_size: n.own_allocated_size,
            is_directory: n.is_directory,
            hard_link: n.hard_link,
            excluded: n.excluded,
        };
        update(&mut value);
        let r = &mut self.tree.records[id as usize];
        r.parent = value.parent.unwrap_or(NO_PARENT);
        r.device = value.identity.device;
        r.inode = value.identity.inode;
        r.logical_size = value.logical_size;
        r.allocated_size = value.allocated_size;
        r.own_logical_size = value.own_logical_size;
        r.own_allocated_size = value.own_allocated_size;
        r.flags = value.flags();
        Ok(())
    }
}
pub struct NodeMetadata {
    pub parent: Option<u32>,
    pub identity: Identity,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub own_logical_size: u64,
    pub own_allocated_size: u64,
    pub is_directory: bool,
    pub hard_link: bool,
    pub excluded: bool,
}
impl NodeMetadata {
    /// Record flag encoding: kind in bits 0-1, then directory, hard link and excluded.
    fn flags(&self) -> u32 {
        let kind = match self.identity.kind {
            NodeKind::Directory => 0,
            NodeKind::File => 1,
            NodeKind::Symlink => 2,
            NodeKind::Other => 3,
        };
        kind | ((self.is_directory as u32) << 2)
            | ((self.hard_link as u32) << 3)
            | ((self.excluded as u32) << 4)
    }
}
impl Snapshot {
    /// Opaque identity of the first record allocation; records are not contiguous.
    pub fn storage_identity(&self) -> usize {
        self.tree.records.as_ptr() as usize
    }
    /// Whether legacy records retain exact path spellings outside the native-name topology.
    pub fn has_path_overrides(&self) -> bool {
        !self.tree.paths.is_empty()
    }
    /// Restore the serialized header root without rewriting any recorded native path.
    /// Validation of the header against node zero remains the caller's responsibility.
    pub fn restore_legacy_root(&mut self, root: PathBuf) -> Result<(), String> {
        if self.root.as_os_str() != root.as_os_str() && self.node_count() > 0 {
            let recorded_root = self.path(0)?;
            if lookup(&self.tree.paths, 0).is_none() {
                self.tree.paths.insert(0, (0, recorded_root));
            }
        }
        self.root = root;
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_node(&mut self, id: u32, n: Node) -> Result<(), String> {
        if id as usize >= self.node_count() {
            return Err("invalid node id".into());
        }
        self.update_node(id, |m| {
            m.parent = n.parent;
            m.identity = n.identity.clone();
            m.logical_size = n.logical_size;
            m.allocated_size = n.allocated_size;
            m.own_logical_size = n.own_logical_size;
            m.own_allocated_size = n.own_allocated_size;
            m.is_directory = n.is_directory;
            m.hard_link = n.hard_link;
            m.excluded = n.excluded;
        })?;
        let bytes = n
            .path
            .file_name()
            .unwrap_or(n.path.as_os_str())
            .as_encoded_bytes();
        let r = &mut self.tree.records[id as usize];
        r.name_offset = self.tree.names.len() as u64;
        r.name_len = u32::try_from(bytes.len()).map_err(|_| "name overflow")?;
        self.tree.names.extend_from_slice(bytes);
        r.children_offset = self.tree.children.len() as u64;
        r.children_len = u32::try_from(n.children.len()).map_err(|_| "child overflow")?;
        self.tree.children.extend(n.children);
        self.tree.clones.retain(|e| e.0 != id);
        if let Some(metadata) = n.clone_metadata.clone() {
            self.tree.clones.push((id, metadata));
            self.tree.clones.sort_by_key(|e| e.0);
        }
        self.tree.paths.retain(|e| e.0 != id);
        self.tree.paths.push((id, n.path));
        self.tree.paths.sort_by_key(|e| e.0);
        self.tree.labels.retain(|e| e.0 != id);
        self.tree.labels.push((id, n.name));
        self.tree.labels.sort_by_key(|e| e.0);
        Ok(())
    }
}
impl serde::Serialize for Snapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeSeq, SerializeStruct};
        struct Nodes<'a>(&'a Snapshot);
        impl serde::Serialize for Nodes<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut seq = s.serialize_seq(Some(self.0.node_count()))?;
                for id in 0..self.0.node_count() {
                    seq.serialize_element(
                        &self
                            .0
                            .legacy_node(id as u32)
                            .map_err(serde::ser::Error::custom)?,
                    )?;
                }
                seq.end()
            }
        }
        let mut s = serializer.serialize_struct("Snapshot", 11)?;
        s.serialize_field("root", &self.root)?;
        s.serialize_field("nodes", &Nodes(self))?;
        s.serialize_field("stats", &self.stats)?;
        s.serialize_field("state", &self.state)?;
        s.serialize_field("errors", &self.errors)?;
        s.serialize_field("exclusions", &self.exclusions)?;
        s.serialize_field("worker_stats", &self.worker_stats)?;
        s.serialize_field("finalization_us", &self.finalization_us)?;
        s.serialize_field("descriptor_limit", &self.descriptor_limit)?;
        s.serialize_field("descriptor_peak", &self.descriptor_peak)?;
        s.serialize_field("accounting", &self.accounting)?;
        s.end()
    }
}
impl<'de> serde::Deserialize<'de> for Snapshot {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
        struct Nodes<'a>(&'a mut Snapshot);
        impl<'de> DeserializeSeed<'de> for Nodes<'_> {
            type Value = ();
            fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                struct List<'a>(&'a mut Snapshot);
                impl<'de> Visitor<'de> for List<'_> {
                    type Value = ();
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("legacy scan node sequence")
                    }
                    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                        let mut builder = LegacyTreeBuilder::new(self.0.root.clone());
                        while let Some(n) = seq.next_element::<Node>()? {
                            builder.append(n).map_err(Error::custom)?;
                        }
                        let decoded = builder.finish();
                        self.0.tree = decoded.tree;
                        self.0.root = decoded.root;
                        Ok(())
                    }
                }
                d.deserialize_seq(List(self.0))
            }
        }
        struct Envelope;
        impl<'de> Visitor<'de> for Envelope {
            type Value = Snapshot;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("scan snapshot")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Snapshot, A::Error> {
                let root = seq
                    .next_element::<PathBuf>()?
                    .ok_or_else(|| Error::invalid_length(0, &self))?;
                let mut s = Snapshot::empty(root.clone());
                seq.next_element_seed(Nodes(&mut s))?
                    .ok_or_else(|| Error::invalid_length(1, &self))?;
                s.stats = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(2, &self))?;
                s.state = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(3, &self))?;
                s.errors = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(4, &self))?;
                s.exclusions = seq.next_element()?.unwrap_or_default();
                s.worker_stats = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(6, &self))?;
                s.finalization_us = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(7, &self))?;
                s.descriptor_limit = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(8, &self))?;
                s.descriptor_peak = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(9, &self))?;
                s.accounting = seq.next_element()?.unwrap_or_default();
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(Error::invalid_length(11, &self));
                }
                s.restore_legacy_root(root).map_err(Error::custom)?;
                Ok(s)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Snapshot, A::Error> {
                let mut s = Snapshot::empty(PathBuf::new());
                let mut seen = std::collections::HashSet::new();
                let mut root = None;
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(Error::custom("duplicate snapshot field"));
                    }
                    match key.as_str() {
                        "root" => {
                            let value: PathBuf = map.next_value()?;
                            if s.node_count() == 0 {
                                s.root = value.clone();
                            }
                            root = Some(value);
                        }
                        "nodes" => map.next_value_seed(Nodes(&mut s))?,
                        "accounting" => s.accounting = map.next_value()?,
                        "stats" => s.stats = map.next_value()?,
                        "state" => s.state = map.next_value()?,
                        "errors" => s.errors = map.next_value()?,
                        "exclusions" => s.exclusions = map.next_value()?,
                        "worker_stats" => s.worker_stats = map.next_value()?,
                        "finalization_us" => s.finalization_us = map.next_value()?,
                        "descriptor_limit" => s.descriptor_limit = map.next_value()?,
                        "descriptor_peak" => s.descriptor_peak = map.next_value()?,
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                for field in [
                    "root",
                    "nodes",
                    "stats",
                    "state",
                    "errors",
                    "worker_stats",
                    "finalization_us",
                ] {
                    if !seen.contains(field) {
                        return Err(Error::custom(format!("missing field {field}")));
                    }
                }
                let root = root.ok_or_else(|| Error::custom("missing root"))?;
                s.restore_legacy_root(root).map_err(Error::custom)?;
                Ok(s)
            }
        }
        d.deserialize_struct(
            "Snapshot",
            &[
                "root",
                "nodes",
                "stats",
                "state",
                "errors",
                "exclusions",
                "worker_stats",
                "finalization_us",
                "descriptor_limit",
                "descriptor_peak",
                "accounting",
            ],
            Envelope,
        )
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn dto(id: u32, parent: Option<u32>, path: PathBuf, directory: bool) -> Node {
        Node {
            clone_metadata: None,
            id,
            parent,
            path: path.clone(),
            name: path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            children: vec![],
            identity: Identity {
                device: 4,
                inode: u64::from(id) + 1,
                kind: if directory {
                    NodeKind::Directory
                } else {
                    NodeKind::File
                },
            },
            logical_size: 7,
            allocated_size: 8,
            own_logical_size: 9,
            own_allocated_size: 10,
            is_directory: directory,
            hard_link: false,
            excluded: false,
        }
    }
    #[test]
    fn sibling_run_rollup_matches_original_across_chunks_and_saturation() {
        let count = records::CHUNK_SIZE * 2 + 17;
        for case in 0..8 {
            let parents: Vec<_> = (0..count)
                .map(|id| {
                    if id == 0 {
                        return if case == 7 {
                            (count - 1) as u32
                        } else {
                            NO_PARENT
                        };
                    }
                    match case {
                        0 => 0,
                        1 => (id - 1) as u32,
                        2 => (id / 17) as u32,
                        3 if id > 1 => (id % 2) as u32,
                        4 => (id / 64) as u32,
                        5 if id % 11 == 0 => NO_PARENT,
                        5 => ((id as u64).wrapping_mul(1_103_515_245) as usize % id) as u32,
                        6 if id == count - 5 => (count + 99) as u32,
                        6 if id % 19 == 0 => id as u32,
                        6 if id % 23 == 0 => (id + 1).min(count - 1) as u32,
                        _ => 0,
                    }
                })
                .collect();
            let mut actual = rollup_fixture(&parents, case);
            let mut expected = actual.clone();
            for repetition in 0..2 {
                assert_eq!(
                    actual.aggregate_sizes_to_parents(),
                    original_size_rollup(&mut expected),
                    "case {case}, repetition {repetition}"
                );
                assert_rollup_records_equal(&actual, &expected);
            }
        }
    }
    #[test]
    fn sibling_run_rollup_matches_original_for_all_small_parent_graphs() {
        const COUNT: usize = 4;
        let choices = [NO_PARENT, 0, 1, 2, 3, 4, 5];
        for mut code in 0usize..choices.len().pow(COUNT as u32) {
            let mut parents = [NO_PARENT; COUNT];
            for parent in &mut parents {
                *parent = choices[code % choices.len()];
                code /= choices.len();
            }
            let mut actual = rollup_fixture(&parents, 3);
            let mut expected = actual.clone();
            assert_eq!(
                actual.aggregate_sizes_to_parents(),
                original_size_rollup(&mut expected)
            );
            assert_rollup_records_equal(&actual, &expected);
        }
    }
    fn rollup_fixture(parents: &[u32], salt: u64) -> Snapshot {
        let mut snapshot = Snapshot::empty("/rollup".into());
        let sizes = [0, 1, 17, u64::MAX - 1, u64::MAX];
        for (id, parent) in parents.iter().enumerate() {
            snapshot.tree.records.push(Record {
                device: 7,
                inode: id as u64 + 1,
                logical_size: sizes[(id + salt as usize) % sizes.len()],
                allocated_size: sizes[(id * 3 + salt as usize) % sizes.len()],
                own_logical_size: id as u64,
                own_allocated_size: id as u64 * 2,
                name_offset: id as u64,
                children_offset: id as u64,
                parent: *parent,
                name_len: 1,
                children_len: 0,
                flags: id as u32 % 32,
            });
        }
        snapshot
    }
    // Frozen original one-record reverse-ID algorithm: independent of run grouping.
    fn original_size_rollup(s: &mut Snapshot) -> Result<(), String> {
        for id in (0..s.node_count()).rev() {
            let r = &s.tree.records[id];
            let (parent, logical, allocated) = (r.parent, r.logical_size, r.allocated_size);
            if parent != NO_PARENT {
                let parent = s
                    .tree
                    .records
                    .get_mut(parent as usize)
                    .ok_or("invalid node id")?;
                parent.logical_size = parent.logical_size.saturating_add(logical);
                parent.allocated_size = parent.allocated_size.saturating_add(allocated);
            }
        }
        Ok(())
    }
    fn assert_rollup_records_equal(actual: &Snapshot, expected: &Snapshot) {
        assert_eq!(actual.node_count(), expected.node_count());
        for id in 0..actual.node_count() {
            assert_eq!(
                format!("{:?}", actual.tree.records[id]),
                format!("{:?}", expected.tree.records[id]),
                "parents/results differ at node {id}"
            );
        }
    }
    #[test]
    fn size_rollup_preserves_general_update_behavior_for_malformed_parents() {
        for parent in [None, Some(0), Some(1), Some(2), Some(99)] {
            let mut actual = Snapshot::empty("/fixture".into());
            for id in 0..3 {
                actual
                    .append_legacy_node(dto(
                        id,
                        (id != 0).then_some(0),
                        if id == 0 {
                            "/fixture".into()
                        } else {
                            format!("/fixture/{id}").into()
                        },
                        id == 0,
                    ))
                    .unwrap();
            }
            actual.update_node(1, |n| n.parent = parent).unwrap();
            let mut expected = actual.clone();
            let reference = (|| -> Result<(), String> {
                for id in (0..expected.node_count()).rev() {
                    let n = expected.node(id as u32).unwrap();
                    if let Some(parent) = n.parent {
                        let (logical, allocated) = (n.logical_size, n.allocated_size);
                        expected.update_node(parent, |n| {
                            n.logical_size = n.logical_size.saturating_add(logical);
                            n.allocated_size = n.allocated_size.saturating_add(allocated);
                        })?;
                    }
                }
                Ok(())
            })();
            assert_eq!(actual.aggregate_sizes_to_parents(), reference);
            // Compare records directly: malformed ancestry need not serialize.
            for id in 0..actual.node_count() {
                assert_eq!(
                    format!("{:?}", actual.tree.records[id]),
                    format!("{:?}", expected.tree.records[id]),
                    "parent {parent:?}, node {id}"
                );
            }
        }
    }
    #[test]
    fn chunk_boundaries_preserve_native_topology_and_clone_metadata() {
        use std::os::unix::ffi::OsStringExt;
        let root = PathBuf::from("/chunk-fixture");
        let mut snapshot = Snapshot::empty(root.clone());
        let mut root_node = dto(0, None, root.clone(), true);
        root_node.children = (1..8200).collect();
        snapshot.append_legacy_node(root_node).unwrap();
        for id in 1..8200 {
            let mut name = vec![0xfe, b'n'];
            name.extend_from_slice(id.to_string().as_bytes());
            let mut node = dto(
                id,
                Some(0),
                root.join(std::ffi::OsString::from_vec(name)),
                false,
            );
            if id == 4096 {
                node.clone_metadata = Some(crate::CloneMetadata {
                    clone_id: 9,
                    data_allocated_size: 8,
                    reference_count: 2,
                    data_logical_size: 7,
                    may_share_blocks: true,
                    verified: true,
                });
            }
            snapshot.append_legacy_node(node).unwrap();
        }
        for id in [4095, 4096, 4097, 8191, 8192] {
            let node = snapshot.node(id).unwrap();
            assert_eq!(node.parent, Some(0));
            assert_eq!(node.identity.inode, u64::from(id) + 1);
            assert_eq!(
                node.path().unwrap().file_name().unwrap(),
                node.native_name()
            );
            assert_eq!(node.own_allocated_size, 10);
        }
        assert_eq!(
            snapshot
                .node(4096)
                .unwrap()
                .clone_metadata()
                .unwrap()
                .clone_id,
            9
        );
        assert_eq!(
            snapshot.children(0).unwrap(),
            &(1..8200).collect::<Vec<_>>()
        );
        let copy = snapshot.clone();
        assert_eq!(copy.path(8192).unwrap(), snapshot.path(8192).unwrap());
        assert_ne!(copy.storage_identity(), snapshot.storage_identity());
    }
    #[test]
    fn compact_layout_and_full_path_oracle() {
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(std::mem::size_of::<Record>(), 80);
        let root = PathBuf::from("/fixture/root/");
        let mut s = Snapshot::empty(root.clone());
        let mut oracle = vec![dto(0, None, root, true)];
        for id in 1..400u32 {
            let parent = if id < 200 { 0 } else { id - 1 };
            let mut bytes = vec![b'n', 0xfe];
            bytes.extend_from_slice(id.to_string().as_bytes());
            let name = std::ffi::OsString::from_vec(bytes);
            let path = oracle[parent as usize].path.join(name);
            oracle.push(dto(id, Some(parent), path, true));
            oracle[parent as usize].children.push(id);
        }
        for n in &oracle {
            s.append_legacy_node(n.clone()).unwrap();
        }
        let mut scratch = PathBuf::new();
        let mut cursor = s.path_cursor();
        for n in &oracle {
            let (path, parent) = cursor.paths(n.id).unwrap();
            assert_eq!(path.as_os_str(), n.path.as_os_str());
            assert_eq!(
                parent.map(Path::as_os_str),
                n.parent.map(|id| oracle[id as usize].path.as_os_str())
            );
            s.path_into(n.id, &mut scratch).unwrap();
            assert_eq!(scratch.as_os_str(), n.path.as_os_str());
            let actual = s.legacy_node(n.id).unwrap();
            assert_eq!(actual.identity, n.identity);
            assert_eq!(actual.children, n.children);
            assert_eq!(actual.name, n.name);
            assert_eq!(actual.own_allocated_size, 10);
        }
        let capacity = scratch.capacity();
        s.path_into(399, &mut scratch).unwrap();
        assert_eq!(capacity, scratch.capacity());
        let report = crate::storage_stats(&s);
        assert_eq!(report.path_overrides.capacity_bytes, 0);
        assert_eq!(report.display_name_overrides.capacity_bytes, 0);
        let legacy_capacity = oracle.capacity() * std::mem::size_of::<Node>()
            + oracle
                .iter()
                .map(|n| n.path.capacity() + n.name.capacity() + n.children.capacity() * 4)
                .sum::<usize>();
        assert!(
            report.retained_capacity_bytes * 100 <= legacy_capacity * 65,
            "compact={} legacy={}",
            report.retained_capacity_bytes,
            legacy_capacity
        );
    }

    #[test]
    fn legacy_paths_without_basename_keep_original_bytes() {
        for (root, child) in [("/", "/.."), ("/a", "/a/.."), ("/a", "/")] {
            let mut parent = dto(0, None, root.into(), true);
            parent.children = vec![1];
            let leaf = dto(1, Some(0), child.into(), true);
            let original = vec![parent, leaf];
            let mut direct = Snapshot::empty(root.into());
            let mut builder = LegacyTreeBuilder::new(root.into());
            for node in &original {
                direct.append_legacy_node(node.clone()).unwrap();
                builder.append(node.clone()).unwrap();
            }
            for snapshot in [direct, builder.finish()] {
                assert!(snapshot.has_path_overrides());
                let wire = serde_json::to_value(&snapshot).unwrap();
                assert_eq!(wire["nodes"], serde_json::to_value(&original).unwrap());
                let decoded: Snapshot = serde_json::from_value(wire).unwrap();
                for node in &original {
                    assert_eq!(
                        decoded.path(node.id).unwrap().as_os_str(),
                        node.path.as_os_str()
                    );
                    assert_eq!(decoded.legacy_node(node.id).unwrap().name, node.name);
                    assert_eq!(
                        decoded.path_cursor().path(node.id).unwrap().as_os_str(),
                        node.path.as_os_str()
                    );
                }
            }
        }
    }
    #[test]
    fn path_cursor_reuses_scratch_and_preserves_sparse_parent_spellings() {
        let mut s = Snapshot::empty("/root".into());
        s.append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        s.append_legacy_node(dto(1, Some(0), "/root/./odd".into(), true))
            .unwrap();
        for id in 2..300 {
            s.append_legacy_node(dto(
                id,
                Some(1),
                PathBuf::from(format!("/root/./odd/entry-{id}")),
                false,
            ))
            .unwrap();
        }
        let mut cursor = s.path_cursor();
        for id in (0..300).chain((0..300).rev()) {
            let (path, parent) = cursor.paths(id).unwrap();
            assert_eq!(path.as_os_str(), s.path(id).unwrap().as_os_str());
            assert_eq!(
                parent.map(Path::as_os_str),
                s.node(id)
                    .unwrap()
                    .parent
                    .map(|p| s.path(p).unwrap())
                    .as_deref()
                    .map(Path::as_os_str)
            );
        }
        cursor.path(299).unwrap();
        let capacity = cursor.capacity_bytes();
        for id in 2..300 {
            cursor.path(id).unwrap();
        }
        assert_eq!(cursor.capacity_bytes(), capacity);
        assert!(cursor.path(u32::MAX).is_err());
        drop(cursor);
        s.update_node(1, |n| n.parent = Some(1)).unwrap();
        assert!(s.path_cursor().path(1).is_err());
    }
    #[test]
    fn owning_legacy_builder_matches_uncached_packing_without_retained_scratch() {
        use std::os::unix::ffi::OsStringExt;
        let mut oracle = vec![
            dto(0, None, "/scope//".into(), true),
            dto(1, Some(0), "/scope/./left".into(), true),
            dto(2, Some(0), "/scope/right".into(), true),
        ];
        for id in 3..500u32 {
            let parent = if id % 2 == 0 { 1 } else { 2 };
            let mut bytes = vec![b'n', 0xff];
            bytes.extend_from_slice(id.to_string().as_bytes());
            let rawname = std::ffi::OsString::from_vec(bytes);
            let path = oracle[parent as usize].path.join(rawname);
            let mut n = dto(id, Some(parent), path, false);
            if id % 17 == 0 {
                n.name = "legacy arbitrary label".into();
            }
            oracle.push(n);
            oracle[parent as usize].children.push(id);
        }
        let mut uncached = Snapshot::empty("/scope//".into());
        let mut builder = LegacyTreeBuilder::new(PathBuf::new());
        for n in &oracle {
            uncached.append_legacy_node(n.clone()).unwrap();
            builder.append(n.clone()).unwrap();
        }
        assert_eq!(builder.node_count(), oracle.len());
        assert!(builder.scratch_capacity_bytes() > 0);
        let mut packed = builder.finish();
        packed.restore_legacy_root("/scope".into()).unwrap();
        uncached.restore_legacy_root("/scope".into()).unwrap();
        assert_eq!(
            crate::storage_stats(&packed).retained_capacity_bytes,
            crate::storage_stats(&uncached).retained_capacity_bytes
        );
        for n in oracle {
            let got = packed.legacy_node(n.id).unwrap();
            assert_eq!(got.path.as_os_str(), n.path.as_os_str());
            assert_eq!(got.name, n.name);
            assert_eq!(got.parent, n.parent);
            assert_eq!(got.children, n.children);
            assert_eq!(got.identity, n.identity);
        }
    }
    #[test]
    fn owning_builder_rejects_original_invalid_fields_before_publishing() {
        let mut builder = LegacyTreeBuilder::new(PathBuf::new());
        let mut root = dto(0, None, "/root".into(), true);
        root.is_directory = false;
        assert!(builder.append(root).is_err());
        assert_eq!(builder.node_count(), 0);
        assert!(builder.snapshot().root().as_os_str().is_empty());
        builder.append(dto(0, None, "/root".into(), true)).unwrap();
        for mut bad in [
            dto(1, Some(2), "/root/file".into(), false),
            dto(2, Some(0), "/root/file".into(), false),
            dto(1, None, "/root/file".into(), false),
        ] {
            bad.name = "unchanged label".into();
            assert!(builder.append(bad).is_err());
            assert_eq!(builder.node_count(), 1);
        }
        let mut bad = dto(1, Some(0), "/root/link".into(), false);
        bad.identity.kind = NodeKind::Symlink;
        bad.hard_link = true;
        assert!(builder.append(bad).is_err());
        builder
            .append(dto(1, Some(0), "/other/./file".into(), false))
            .unwrap();
        assert_eq!(
            builder.snapshot().path(1).unwrap().as_os_str(),
            OsStr::new("/other/./file")
        );
    }
    fn short_native_names() -> Vec<Vec<u8>> {
        let alphabet = [b'/', b'.', 0, 0xff, b'\\', b'a'];
        let mut names = Vec::new();
        for len in 0..=5usize {
            for mut number in 0..alphabet.len().pow(len as u32) {
                let mut bytes = vec![0; len];
                for byte in &mut bytes {
                    *byte = alphabet[number % alphabet.len()];
                    number /= alphabet.len();
                }
                names.push(bytes);
            }
        }
        names
    }
    #[test]
    fn unix_byte_component_proof_matches_standard_path_exhaustively() {
        for bytes in short_native_names() {
            let name = OsStr::from_bytes(&bytes);
            let mut components = Path::new(name).components();
            let expected = matches!(components.next(),Some(std::path::Component::Normal(component)) if component==name)
                && components.next().is_none();
            assert_eq!(is_exact_native_component(name), expected, "name={bytes:?}");
        }
    }
    #[test]
    fn unix_byte_join_matches_standard_path_for_exact_components_and_prefixes() {
        let prefixes: [&[u8]; 22] = [
            b"",
            b".",
            b"..",
            b"./",
            b"../",
            b"foo",
            b"foo/",
            b"foo//",
            b"foo/.",
            b"foo/..",
            b"/",
            b"//",
            b"///",
            b"/foo",
            b"/foo/",
            b"/foo//",
            b"/foo/.",
            b"/foo/..",
            b"C:",
            b"native\0prefix",
            b"raw\xff/",
            b"back\\slash",
        ];
        let names = short_native_names();
        let mut output = PathBuf::new();
        for prefix in prefixes {
            let parent = Path::new(OsStr::from_bytes(prefix));
            for name in names
                .iter()
                .filter(|name| is_exact_native_component(OsStr::from_bytes(name)))
            {
                let name = OsStr::from_bytes(name);
                let expected = parent.join(name);
                join_native_component_into(parent, name, &mut output).unwrap();
                assert_eq!(
                    output.as_os_str(),
                    expected.as_os_str(),
                    "prefix={prefix:?} name={name:?}"
                );
            }
        }
        let capacity = output.capacity();
        for _ in 0..5 {
            join_native_component_into(Path::new("/"), OsStr::new("a"), &mut output).unwrap();
        }
        assert_eq!(output.capacity(), capacity);
    }
    #[test]
    fn cursor_byte_join_preserves_old_fallback_for_malformed_private_names() {
        let names = short_native_names();
        for root in [
            "",
            ".",
            "..",
            "relative//",
            "/",
            "//",
            "/root/..",
            "/root//",
        ] {
            let mut s = Snapshot::empty(root.into());
            s.append_legacy_node(dto(0, None, root.into(), true))
                .unwrap();
            s.append_legacy_node(dto(1, Some(0), Path::new(root).join("seed"), false))
                .unwrap();
            for bytes in &names {
                let start = s.tree.names.len() as u64;
                s.tree.names.extend_from_slice(bytes);
                s.tree.records[1].name_offset = start;
                s.tree.records[1].name_len = bytes.len() as u32;
                let name = OsStr::from_bytes(bytes);
                let mut components = Path::new(name).components();
                let old_ordinary =
                    matches!(components.next(), Some(std::path::Component::Normal(_)))
                        && components.next().is_none();
                let expected = if old_ordinary {
                    Path::new(root).join(name)
                } else {
                    s.path(1).unwrap()
                };
                assert_eq!(
                    s.path_cursor().path(1).unwrap().as_os_str(),
                    expected.as_os_str(),
                    "root={root:?} name={bytes:?}"
                );
            }
            s.tree.records[1].name_offset = u64::MAX;
            assert!(s.path_cursor().path(1).is_err());
        }
        for (root, path) in [
            ("/", "/.."),
            ("/a", "/a/.."),
            ("/a", "/"),
            ("/a//", "/a/./historical"),
        ] {
            let mut builder = LegacyTreeBuilder::new(root.into());
            builder.append(dto(0, None, root.into(), true)).unwrap();
            builder.append(dto(1, Some(0), path.into(), true)).unwrap();
            let s = builder.finish();
            assert!(s.has_path_overrides());
            assert_eq!(
                s.path_cursor().path(1).unwrap().as_os_str(),
                OsStr::new(path)
            );
        }
    }
    #[test]
    fn native_component_checks_exact_bytes_without_changing_nul_policy() {
        let mut s = Snapshot::empty("/root".into());
        s.append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        s.append_legacy_node(dto(1, Some(0), "/root/child".into(), false))
            .unwrap();
        for bytes in [
            b"".as_slice(),
            b".",
            b"..",
            b"foo/",
            b"/foo",
            b"a/b",
            b"./foo",
            b"foo//",
        ] {
            let offset = s.tree.names.len() as u64;
            s.tree.names.extend_from_slice(bytes);
            s.tree.records[1].name_offset = offset;
            s.tree.records[1].name_len = bytes.len() as u32;
            assert!(s.native_component(1).is_err(), "name={bytes:?}");
        }
        let bytes = b"native\0name";
        let offset = s.tree.names.len() as u64;
        s.tree.names.extend_from_slice(bytes);
        s.tree.records[1].name_offset = offset;
        s.tree.records[1].name_len = bytes.len() as u32;
        assert_eq!(s.native_component(1).unwrap().as_bytes(), bytes);
        s.tree.records[1].name_offset = u64::MAX;
        assert!(s.native_component(1).is_err());
    }
    #[test]
    fn direct_scanned_metadata_matches_legacy_paths_labels_and_all_fields() {
        use std::os::unix::ffi::OsStringExt;
        for root in [
            PathBuf::from("/"),
            PathBuf::from("/fixture/root/"),
            PathBuf::from("relative/root"),
        ] {
            let mut direct = Snapshot::empty(root.clone());
            let mut legacy = Snapshot::empty(root.clone());
            let paths = [
                root.clone(),
                root.join("nested"),
                root.join("ordinary"),
                root.join(std::ffi::OsString::from_vec(b"raw\xff".to_vec())),
                root.join(std::ffi::OsString::from_vec(b"raw\xfe".to_vec())),
            ];
            for (id, path) in paths.iter().enumerate() {
                let directory = id < 2;
                let mut n = dto(
                    id as u32,
                    if id == 0 { None } else { Some(0) },
                    path.clone(),
                    directory,
                );
                if id == 3 {
                    n.identity.kind = NodeKind::Symlink;
                }
                n.identity.device = u64::MAX - 5;
                n.identity.inode = u64::MAX - id as u64;
                n.logical_size = 100 + id as u64;
                n.allocated_size = 200 + id as u64;
                n.own_logical_size = 300 + id as u64;
                n.own_allocated_size = 400 + id as u64;
                n.hard_link = id == 2;
                n.excluded = id == 1;
                let metadata = NodeMetadata {
                    parent: n.parent,
                    identity: n.identity.clone(),
                    logical_size: n.logical_size,
                    allocated_size: n.allocated_size,
                    own_logical_size: n.own_logical_size,
                    own_allocated_size: n.own_allocated_size,
                    is_directory: n.is_directory,
                    hard_link: n.hard_link,
                    excluded: n.excluded,
                };
                direct
                    .append_scanned_node(&crate::ScannedNode {
                        id: n.id,
                        path: path.clone().into(),
                        metadata,
                    })
                    .unwrap();
                legacy.append_legacy_node(n).unwrap();
            }
            direct.set_children(0, &[4, 1, 3, 2]).unwrap();
            legacy.set_children(0, &[4, 1, 3, 2]).unwrap();
            for id in 0..5 {
                let got = direct.legacy_node(id).unwrap();
                let expected = legacy.legacy_node(id).unwrap();
                assert_eq!(got.path.as_os_str(), expected.path.as_os_str());
                assert_eq!(got.name, expected.name);
                assert_eq!(got.identity, expected.identity);
                assert_eq!(got.parent, expected.parent);
                assert_eq!(got.children, expected.children);
                assert_eq!(
                    (
                        got.logical_size,
                        got.allocated_size,
                        got.own_logical_size,
                        got.own_allocated_size
                    ),
                    (
                        expected.logical_size,
                        expected.allocated_size,
                        expected.own_logical_size,
                        expected.own_allocated_size
                    )
                );
                assert_eq!(
                    (got.is_directory, got.hard_link, got.excluded),
                    (expected.is_directory, expected.hard_link, expected.excluded)
                );
            }
            assert_eq!(
                crate::storage_stats(&direct)
                    .display_name_overrides
                    .capacity_bytes,
                0
            );
        }
    }
    #[test]
    fn borrowed_scanned_publication_keeps_validation_and_owns_retained_names() {
        fn scanned(n: &Node) -> crate::ScannedNode {
            crate::ScannedNode {
                id: n.id,
                path: n.path.clone().into(),
                metadata: NodeMetadata {
                    parent: n.parent,
                    identity: n.identity.clone(),
                    logical_size: n.logical_size,
                    allocated_size: n.allocated_size,
                    own_logical_size: n.own_logical_size,
                    own_allocated_size: n.own_allocated_size,
                    is_directory: n.is_directory,
                    hard_link: n.hard_link,
                    excluded: n.excluded,
                },
            }
        }
        for has_root in [false, true] {
            let mut base = Snapshot::empty("/root".into());
            if has_root {
                base.append_legacy_node(dto(0, None, "/root".into(), true))
                    .unwrap();
            }
            let id = u32::from(has_root);
            let parent = has_root.then_some(0);
            let valid = dto(id, parent, "/root/file".into(), false);
            let mut invalid = Vec::new();
            let mut n = valid.clone();
            n.id = id + 1;
            invalid.push(n);
            for parent in [None, Some(0), Some(id), Some(id + 1), Some(u32::MAX)] {
                if parent != valid.parent {
                    let mut n = valid.clone();
                    n.parent = parent;
                    invalid.push(n);
                }
            }
            let mut n = valid.clone();
            n.is_directory = true;
            invalid.push(n);
            for kind in [NodeKind::Directory, NodeKind::Symlink, NodeKind::Other] {
                let mut n = valid.clone();
                n.identity.kind = kind;
                n.is_directory = kind == NodeKind::Directory;
                n.hard_link = true;
                invalid.push(n);
            }
            for n in invalid {
                let mut direct = base.clone();
                let mut legacy = base.clone();
                let before = serde_json::to_value(&direct).unwrap();
                let result = direct.append_scanned_node(&scanned(&n));
                assert!(result.is_err(), "accepted {n:?}");
                assert_eq!(result, legacy.append_legacy_node(n));
                assert_eq!(serde_json::to_value(&direct).unwrap(), before);
            }
        }

        let mut s = Snapshot::empty("/root".into());
        s.append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        // Exercise inline and owned temporary names; neither may be borrowed by the tree.
        for (id, path) in ["short", "/root/a-longer-than-twenty-two-byte-owned-name"]
            .into_iter()
            .enumerate()
        {
            let id = id as u32 + 1;
            let mut source = scanned(&dto(id, Some(0), path.into(), false));
            let expected = source.path.native_name_or_path().to_owned();
            s.append_scanned_node(&source).unwrap();
            source.path = "replacement".into();
            source.metadata.own_logical_size = u64::MAX;
            drop(source);
            assert_eq!(s.native_name(id).unwrap(), expected);
            assert_eq!(s.node(id).unwrap().own_logical_size, 9);
        }
    }
    #[test]
    fn scanned_child_ranges_match_slice_publication_across_chunks_and_empty_prefixes() {
        let mut direct = Snapshot::empty("/root".into());
        direct
            .append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        for id in 1..=3 {
            direct
                .append_legacy_node(dto(id, Some(0), format!("/root/d{id}").into(), true))
                .unwrap();
        }
        direct.set_children(0, &[1, 2, 3]).unwrap();
        let mut slices = direct.clone();
        for (parent, count) in [(1, records::CHUNK_SIZE + 13), (2, 17), (3, 0)] {
            let first_child = direct.node_count();
            let mut ids = Vec::new();
            for index in 0..count {
                let id = direct.node_count() as u32;
                let mut child = dto(
                    id,
                    Some(parent),
                    format!("/root/d{parent}/f{index}").into(),
                    false,
                );
                child.excluded = index % 3 == 0;
                ids.push(id);
                direct.append_legacy_node(child.clone()).unwrap();
                slices.append_legacy_node(child).unwrap();
            }
            direct.set_scanned_children(parent, first_child).unwrap();
            slices.set_children(parent, &ids).unwrap();
            assert_eq!(direct.children(parent).unwrap(), ids);
        }
        for id in 0..direct.node_count() as u32 {
            let actual = direct.legacy_node(id).unwrap();
            let expected = slices.legacy_node(id).unwrap();
            assert_eq!(actual.children, expected.children);
            assert_eq!(actual.parent, expected.parent);
            assert_eq!(actual.identity, expected.identity);
            assert_eq!(actual.path, expected.path);
            assert_eq!(actual.excluded, expected.excluded);
            assert_eq!(actual.own_allocated_size, expected.own_allocated_size);
        }
        // Empty/error listings replace an old child slice without affecting any
        // nodes; cancellation similarly publishes only the successfully appended prefix.
        direct.set_scanned_children(1, direct.node_count()).unwrap();
        assert!(direct.children(1).unwrap().is_empty());
        let adjacency = direct.tree.children.clone();
        for first_child in [0, direct.node_count() + 1, usize::MAX] {
            assert!(direct.set_scanned_children(0, first_child).is_err());
            assert_eq!(direct.tree.children, adjacency);
            assert_eq!(direct.children(0).unwrap(), [1, 2, 3]);
        }
    }
    #[test]
    fn borrowed_builder_copies_reusable_wire_record_before_next_mutation() {
        use std::os::unix::ffi::OsStringExt;
        let mut builder = LegacyTreeBuilder::new(PathBuf::new());
        let mut owned = LegacyTreeBuilder::new(PathBuf::new());
        let mut wire = dto(0, None, "/scope//".into(), true);
        wire.name = "root label".into();
        wire.children = vec![1, 2, 3];
        let mut oracle = vec![];
        for id in 0..4 {
            if id > 0 {
                wire.id = id;
                wire.parent = Some(0);
                wire.children.clear();
                wire.path = match id {
                    1 => PathBuf::from("/scope/./odd"),
                    2 => PathBuf::from("/scope/.."),
                    _ => {
                        Path::new("/scope").join(std::ffi::OsString::from_vec(b"raw\xff".to_vec()))
                    }
                };
                wire.identity.kind = NodeKind::File;
                wire.identity.inode = id as u64 + 10;
                wire.is_directory = false;
                wire.hard_link = id == 3;
                wire.name = format!("independent legacy label {id}");
                wire.logical_size = id as u64 + 1;
                wire.allocated_size = id as u64 + 2;
                wire.own_logical_size = id as u64 + 3;
                wire.own_allocated_size = id as u64 + 4;
            }
            oracle.push(wire.clone());
            builder.append_ref(&wire).unwrap();
            owned.append(wire.clone()).unwrap();
        }
        wire.path.clear();
        wire.name.clear();
        wire.children.clear();
        wire.identity.inode = 0;
        wire.logical_size = 0;
        let got = builder.finish();
        let expected = owned.finish();
        for n in oracle {
            let actual = got.legacy_node(n.id).unwrap();
            assert_eq!(actual.path.as_os_str(), n.path.as_os_str());
            assert_eq!(actual.name, n.name);
            assert_eq!(actual.children, n.children);
            let other = expected.legacy_node(n.id).unwrap();
            assert_eq!(actual.identity, other.identity);
            assert_eq!(actual.parent, other.parent);
            assert_eq!(
                (
                    actual.logical_size,
                    actual.allocated_size,
                    actual.own_logical_size,
                    actual.own_allocated_size
                ),
                (
                    other.logical_size,
                    other.allocated_size,
                    other.own_logical_size,
                    other.own_allocated_size
                )
            );
            assert_eq!(
                (actual.is_directory, actual.hard_link, actual.excluded),
                (other.is_directory, other.hard_link, other.excluded)
            );
        }
        let mut builder = LegacyTreeBuilder::new(PathBuf::new());
        let mut bad = dto(0, None, "/bad".into(), true);
        bad.is_directory = false;
        assert!(builder.append_ref(&bad).is_err());
        assert_eq!(builder.node_count(), 0);
        assert!(builder.snapshot().root.as_os_str().is_empty());
    }
    #[test]
    fn checked_ranges_and_ancestry_fail_closed() {
        let mut s = Snapshot::empty("/root".into());
        s.append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        s.append_legacy_node(dto(1, Some(0), "/root/child".into(), false))
            .unwrap();
        s.tree.records[1].name_offset = u64::MAX;
        assert!(s.native_name(1).is_err());
        assert!(s.path(1).is_err());
        s.tree.records[0].children_offset = u64::MAX;
        s.tree.records[0].children_len = 2;
        assert!(s.children(0).is_err());
        s.tree.records[1].name_offset = 0;
        s.tree.records[1].name_len = 0;
        s.tree.records[1].parent = 1;
        assert!(s.path(1).is_err());
    }
    #[test]
    fn serde_streams_legacy_nodes_in_both_envelope_orders() {
        let mut s = Snapshot::empty("/root".into());
        s.append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        s.append_legacy_node(dto(1, Some(0), "/root/./child".into(), false))
            .unwrap();
        s.set_children(0, &[1]).unwrap();
        let encoded = serde_json::to_value(&s).unwrap();
        let decoded: Snapshot = serde_json::from_value(encoded.clone()).unwrap();
        for id in 0..2 {
            assert_eq!(
                decoded.path(id).unwrap().as_os_str(),
                s.path(id).unwrap().as_os_str()
            );
            assert_eq!(decoded.children(id).unwrap(), s.children(id).unwrap());
        }
        let mut object = encoded.as_object().unwrap().clone();
        let nodes = object.remove("nodes").unwrap();
        let root = object.remove("root").unwrap();
        let mut fields = vec![format!("\"nodes\":{}", nodes), format!("\"root\":{}", root)];
        for (key, value) in object {
            fields.push(format!(
                "{}:{}",
                serde_json::to_string(&key).unwrap(),
                value
            ));
        }
        let reversed: Snapshot =
            serde_json::from_str(&format!("{{{}}}", fields.join(","))).unwrap();
        assert_eq!(
            reversed.path(1).unwrap().as_os_str(),
            s.path(1).unwrap().as_os_str()
        );
        let mut invalid = encoded;
        invalid["nodes"][1]["is_directory"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<Snapshot>(invalid).is_err());
    }
    #[test]
    fn positional_serde_and_legacy_root_spellings_remain_lossless() {
        let fields = [
            "root",
            "nodes",
            "stats",
            "state",
            "errors",
            "exclusions",
            "worker_stats",
            "finalization_us",
            "descriptor_limit",
            "descriptor_peak",
        ];
        for (header, recorded) in [
            ("/scope", "/scope//"),
            ("/scope//", "/scope"),
            ("/scope/./", "/scope//"),
        ] {
            let mut s = Snapshot::empty(recorded.into());
            s.append_legacy_node(dto(0, None, recorded.into(), true))
                .unwrap();
            s.append_legacy_node(dto(1, Some(0), "/scope/child".into(), false))
                .unwrap();
            s.set_children(0, &[1]).unwrap();
            s.restore_legacy_root(header.into()).unwrap();
            assert_eq!(s.path(0).unwrap().as_os_str(), OsStr::new(recorded));
            assert_eq!(s.path(1).unwrap().as_os_str(), OsStr::new("/scope/child"));
            let wire = serde_json::to_value(&s).unwrap();
            let positional =
                serde_json::Value::Array(fields.iter().map(|field| wire[*field].clone()).collect());
            for decoded in [
                serde_json::from_value::<Snapshot>(wire).unwrap(),
                serde_json::from_value::<Snapshot>(positional).unwrap(),
            ] {
                assert_eq!(decoded.root.as_os_str(), OsStr::new(header));
                assert_eq!(decoded.path(0).unwrap().as_os_str(), OsStr::new(recorded));
                assert_eq!(
                    decoded.path(1).unwrap().as_os_str(),
                    OsStr::new("/scope/child")
                );
                assert_eq!(decoded.children(0).unwrap(), &[1]);
            }
        }
    }
    #[test]
    fn serializer_declared_field_count_matches_emitted_fields() {
        use serde::ser::{Impossible, SerializeStruct, Serializer};
        #[derive(Debug)]
        struct CountError;
        impl std::fmt::Display for CountError {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("unsupported serialization or incorrect declared field count")
            }
        }
        impl std::error::Error for CountError {}
        impl serde::ser::Error for CountError {
            fn custom<T: std::fmt::Display>(_: T) -> Self {
                Self
            }
        }
        struct Counter;
        struct Fields {
            declared: usize,
            emitted: usize,
        }
        impl SerializeStruct for Fields {
            type Ok = usize;
            type Error = CountError;
            fn serialize_field<T: serde::Serialize + ?Sized>(
                &mut self,
                _: &'static str,
                _: &T,
            ) -> Result<(), CountError> {
                self.emitted += 1;
                Ok(())
            }
            fn end(self) -> Result<usize, CountError> {
                if self.emitted == self.declared {
                    Ok(self.emitted)
                } else {
                    Err(CountError)
                }
            }
        }
        macro_rules! reject_scalars {($( $method:ident($value:ident:$ty:ty) ),* $(,)?)=>{$(fn $method(self,_:$ty)->Result<usize,CountError> {Err(CountError)})*};}
        // The serializer deliberately does not inspect field values: its contract is
        // format-independent and catches incorrect struct cardinality directly.
        impl Serializer for Counter {
            type Ok = usize;
            type Error = CountError;
            type SerializeSeq = Impossible<usize, CountError>;
            type SerializeTuple = Impossible<usize, CountError>;
            type SerializeTupleStruct = Impossible<usize, CountError>;
            type SerializeTupleVariant = Impossible<usize, CountError>;
            type SerializeMap = Impossible<usize, CountError>;
            type SerializeStruct = Fields;
            type SerializeStructVariant = Impossible<usize, CountError>;
            reject_scalars!(
                serialize_bool(v: bool),
                serialize_i8(v: i8),
                serialize_i16(v: i16),
                serialize_i32(v: i32),
                serialize_i64(v: i64),
                serialize_u8(v: u8),
                serialize_u16(v: u16),
                serialize_u32(v: u32),
                serialize_u64(v: u64),
                serialize_f32(v: f32),
                serialize_f64(v: f64),
                serialize_char(v: char),
                serialize_str(v: &str),
                serialize_bytes(v: &[u8])
            );
            fn serialize_none(self) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_some<T: serde::Serialize + ?Sized>(
                self,
                _: &T,
            ) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_unit(self) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_unit_struct(self, _: &'static str) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_unit_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
            ) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_newtype_struct<T: serde::Serialize + ?Sized>(
                self,
                _: &'static str,
                _: &T,
            ) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_newtype_variant<T: serde::Serialize + ?Sized>(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: &T,
            ) -> Result<usize, CountError> {
                Err(CountError)
            }
            fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, CountError> {
                Err(CountError)
            }
            fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, CountError> {
                Err(CountError)
            }
            fn serialize_tuple_struct(
                self,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeTupleStruct, CountError> {
                Err(CountError)
            }
            fn serialize_tuple_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeTupleVariant, CountError> {
                Err(CountError)
            }
            fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, CountError> {
                Err(CountError)
            }
            fn serialize_struct(
                self,
                name: &'static str,
                len: usize,
            ) -> Result<Fields, CountError> {
                assert_eq!(name, "Snapshot");
                Ok(Fields {
                    declared: len,
                    emitted: 0,
                })
            }
            fn serialize_struct_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeStructVariant, CountError> {
                Err(CountError)
            }
        }
        let s = Snapshot::empty("/root".into());
        assert_eq!(serde::Serialize::serialize(&s, Counter).unwrap(), 11);
    }
    #[test]
    fn sparse_historical_spelling_and_label_are_preserved() {
        let mut s = Snapshot::empty("/root".into());
        s.append_legacy_node(dto(0, None, "/root".into(), true))
            .unwrap();
        let mut n = dto(1, Some(0), "/root/./odd//name".into(), false);
        n.name = "legacy label".into();
        s.append_legacy_node(n.clone()).unwrap();
        assert_eq!(s.path(1).unwrap().as_os_str(), n.path.as_os_str());
        assert_eq!(s.display_name(1).unwrap(), "legacy label");
        let mut contradictory = dto(2, Some(0), "/root/file".into(), false);
        contradictory.is_directory = true;
        assert!(s.append_legacy_node(contradictory).is_err());
        assert_eq!(s.node_count(), 2);
    }
}
