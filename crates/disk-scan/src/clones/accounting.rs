//! Pure clone/hard-link attribution.
#[cfg(unix)]
use crate::NodePathCursor;
use crate::{AccountingModel, NodeKind, Snapshot};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Validated pure attribution result for scanner consumers.
pub struct AllocationPlan {
    pub values: Vec<u64>,
    pub clone_deduction_bytes: u64,
    pub clone_attributions: Vec<(u32, (bool, u64))>,
}

#[cfg(test)]
pub(crate) fn attributed_own_allocations(s: &Snapshot) -> Result<Vec<u64>, String> {
    allocation_plan(s).map(|p| p.values)
}
#[cfg(test)]
pub(crate) fn hardlink_allocations(s: &Snapshot) -> Result<Vec<u64>, String> {
    allocations(s, false).map(|p| p.materialize(s).values)
}
/// Validate metadata and derive hard-link and full-clone attribution without
/// modifying the snapshot or querying the filesystem.
pub fn allocation_plan(s: &Snapshot) -> Result<AllocationPlan, String> {
    internal_allocation_plan(s).map(|p| p.materialize(s))
}

pub(crate) struct InternalAllocationPlan {
    pub clone_deduction_bytes: u64,
    pub clone_attributions: Vec<(u32, (bool, u64))>,
    aliases: Vec<u32>,
}
struct AllocationCursor<'a> {
    aliases: &'a [u32],
    attributions: &'a [(u32, (bool, u64))],
    alias: usize,
    attribution: usize,
}
impl<'a> AllocationCursor<'a> {
    fn new(plan: &'a InternalAllocationPlan) -> Self {
        Self {
            aliases: &plan.aliases,
            attributions: &plan.clone_attributions,
            alias: 0,
            attribution: 0,
        }
    }
    fn value(&mut self, id: u32, mut raw: u64) -> u64 {
        if self.aliases.get(self.alias) == Some(&id) {
            raw = 0;
            self.alias += 1;
        }
        if let Some((claim, (_, deduction))) = self.attributions.get(self.attribution) {
            if *claim == id {
                raw -= *deduction;
                self.attribution += 1;
            }
        }
        raw
    }
}
impl InternalAllocationPlan {
    pub(crate) fn apply_to(&self, s: &mut Snapshot) {
        let mut cursor = AllocationCursor::new(self);
        s.reset_aggregate_sizes(|id, raw| cursor.value(id, raw));
    }
    fn materialize(self, s: &Snapshot) -> AllocationPlan {
        let mut values: Vec<_> = s.iter_nodes().map(|n| n.own_allocated_size).collect();
        let mut cursor = AllocationCursor::new(&self);
        for (id, value) in values.iter_mut().enumerate() {
            *value = cursor.value(id as u32, *value);
        }
        AllocationPlan {
            values,
            clone_deduction_bytes: self.clone_deduction_bytes,
            clone_attributions: self.clone_attributions,
        }
    }
}
pub(crate) fn internal_allocation_plan(s: &Snapshot) -> Result<InternalAllocationPlan, String> {
    allocations(s, true)
}

#[derive(Clone, Copy)]
struct Group {
    first: u32,
    owner: u32,
    verified_owner: Option<u32>,
    consistent: bool,
}

struct OwnerPathScratch<'a> {
    snapshot: &'a Snapshot,
    #[cfg(unix)]
    cursor: NodePathCursor<'a>,
    fallback: PathBuf,
}
impl<'a> OwnerPathScratch<'a> {
    fn new(snapshot: &'a Snapshot) -> Self {
        Self {
            snapshot,
            #[cfg(unix)]
            cursor: snapshot.path_cursor(),
            fallback: PathBuf::new(),
        }
    }
    fn path(&mut self, id: u32) -> Result<&Path, String> {
        #[cfg(unix)]
        match self.cursor.path(id) {
            Ok(path) => Ok(path),
            Err(_) => {
                // Historical exact spellings can bypass malformed ancestry in
                // path_into. Preserve that acceptance and its original errors.
                self.snapshot.path_into(id, &mut self.fallback)?;
                Ok(&self.fallback)
            }
        }
        #[cfg(not(unix))]
        {
            self.snapshot.path_into(id, &mut self.fallback)?;
            Ok(&self.fallback)
        }
    }
}
fn earlier(
    candidate: u32,
    owner: u32,
    paths: &mut [OwnerPathScratch<'_>; 2],
) -> Result<bool, String> {
    if candidate == owner {
        return Ok(false);
    }
    let [candidate_path, owner_path] = paths;
    Ok(candidate_path.path(candidate)? < owner_path.path(owner)?)
}

/// Dense node iteration can advance through validated sparse IDs. Claims are
/// strictly ascending: appended in node order, and `replace_node` re-sorts.
pub(super) struct ClaimCursor<'a> {
    claims: &'a [(u32, crate::CloneMetadata)],
    index: usize,
}
impl<'a> ClaimCursor<'a> {
    pub(super) fn new(claims: &'a [(u32, crate::CloneMetadata)]) -> Self {
        debug_assert!(claims.windows(2).all(|w| w[0].0 < w[1].0));
        Self { claims, index: 0 }
    }
    pub(super) fn get(&mut self, id: u32) -> Option<&'a crate::CloneMetadata> {
        while self.index < self.claims.len() && self.claims[self.index].0 < id {
            self.index += 1;
        }
        self.claims
            .get(self.index)
            .filter(|(claim, _)| *claim == id)
            .map(|(_, metadata)| metadata)
    }
    fn at(&self, index: usize) -> Option<&'a crate::CloneMetadata> {
        Some(&self.claims[index].1)
    }
}

/// Whether `n` agrees with `first`, the first node of its inode: same sizes and claim, and a second node must
/// be a hard link along with the first.
pub(super) fn alias_consistent(
    n: &crate::NodeRef<'_>,
    claim: Option<&crate::CloneMetadata>,
    first: &crate::NodeRef<'_>,
    first_claim: Option<&crate::CloneMetadata>,
) -> bool {
    (first.length, first.own_allocated_size, first_claim) == (n.length, n.own_allocated_size, claim)
        && (n.id == first.id || (n.hard_link && first.hard_link))
}

/// Avoid hashing every file against a tiny, immutable clone-alias index. Keep
/// references to the original entries so duplicate insertion semantics and the
/// map's lifetime and allocation accounting remain unchanged.
pub(super) enum AliasLookup<'a, V> {
    Small([Option<(&'a (u64, u64), &'a V)>; 4]),
    Large(&'a HashMap<(u64, u64), V>),
}
impl<'a, V> AliasLookup<'a, V> {
    pub(super) fn new(aliases: &'a HashMap<(u64, u64), V>) -> Self {
        if aliases.len() > 4 {
            return Self::Large(aliases);
        }
        let mut entries = [None; 4];
        for (slot, entry) in entries.iter_mut().zip(aliases) {
            *slot = Some(entry);
        }
        Self::Small(entries)
    }
    #[inline]
    pub(super) fn get(&self, key: &(u64, u64)) -> Option<&'a V> {
        match self {
            Self::Small(entries) => entries
                .iter()
                .flatten()
                .find_map(|(candidate, value)| (*candidate == key).then_some(*value)),
            Self::Large(aliases) => aliases.get(key),
        }
    }
}

fn collect_hardlinks(
    s: &Snapshot,
    plan: &mut InternalAllocationPlan,
    inodes: &mut HashMap<(u64, u64), u32>,
    paths: &mut [OwnerPathScratch<'_>; 2],
) -> Result<(), String> {
    use std::collections::hash_map::Entry;
    for n in s.iter_nodes().filter(|n| n.hard_link) {
        let loser = match inodes.entry((n.identity.device, n.identity.inode)) {
            Entry::Vacant(entry) => {
                entry.insert(n.id);
                continue;
            }
            Entry::Occupied(mut entry) => {
                let owner = *entry.get();
                if earlier(n.id, owner, paths)? {
                    entry.insert(n.id);
                    owner
                } else {
                    n.id
                }
            }
        };
        plan.aliases.push(loser);
    }
    // Replaced owners can precede aliases already recorded for other inodes.
    plan.aliases.sort_unstable();
    Ok(())
}

fn allocations(s: &Snapshot, clones: bool) -> Result<InternalAllocationPlan, String> {
    let mut plan = InternalAllocationPlan {
        aliases: Vec::new(),
        clone_deduction_bytes: 0,
        clone_attributions: Vec::new(),
    };
    let mut paths = [OwnerPathScratch::new(s), OwnerPathScratch::new(s)];
    let mut inodes = HashMap::new();
    collect_hardlinks(s, &mut plan, &mut inodes, &mut paths)?;
    if !clones {
        return Ok(plan);
    }
    if s.accounting.model != AccountingModel::FullClonesV1 {
        if !s.tree.clones.is_empty() {
            return Err("legacy accounting contains clone metadata".into());
        }
        return Ok(plan);
    }
    if s.tree.clones.is_empty() {
        return Ok(plan);
    }
    // Index only cloned inodes. References into the immutable snapshot avoid
    // copying allocation and metadata fields into each map entry.
    let mut aliases = HashMap::new();
    for (index, (id, m)) in s.tree.clones.iter().enumerate() {
        let n = s.node(*id).ok_or("invalid clone node")?;
        if n.identity.kind != NodeKind::File
            || m.data_allocated_size > n.own_allocated_size
            || m.data_length > n.length
            || m.clone_id == 0
            || m.reference_count <= 1
        {
            return Err("invalid clone metadata".into());
        }
        aliases
            .entry((n.identity.device, n.identity.inode))
            .or_insert(index);
    }
    let alias_lookup = AliasLookup::new(&aliases);
    let mut claims = ClaimCursor::new(&s.tree.clones);
    for n in s.iter_nodes().filter(|n| n.identity.kind == NodeKind::File) {
        let current = claims.get(n.id);
        if let Some(first) = alias_lookup.get(&(n.identity.device, n.identity.inode)) {
            let first_id = s.tree.clones[*first].0;
            let a = s.node(first_id).ok_or("invalid clone alias")?;
            if !alias_consistent(&n, current, &a, claims.at(*first)) {
                return Err("inconsistent cloned inode aliases or link flags".into());
            }
        }
    }
    // Alias consistency is established before grouping. Release its map so
    // these two sparse indexes never overlap.
    drop(aliases);
    let mut groups: HashMap<(u64, u64), Group> = HashMap::new();
    for (id, m) in &s.tree.clones {
        let n = s.node(*id).ok_or("invalid clone node")?;
        if n.hard_link && inodes.get(&(n.identity.device, n.identity.inode)) != Some(id) {
            continue;
        }
        let group = groups
            .entry((n.identity.device, m.clone_id))
            .or_insert(Group {
                first: *id,
                owner: *id,
                verified_owner: None,
                consistent: true,
            });
        let first = s.node(group.first).ok_or("invalid clone group")?;
        let first_metadata = first.clone_metadata().ok_or("missing clone metadata")?;
        group.consistent &= m.verified
            && m.data_allocated_size == first_metadata.data_allocated_size
            && m.data_length == first_metadata.data_length;
        let previous_owner = group.owner;
        let replaces_owner = earlier(*id, previous_owner, &mut paths)?;
        if replaces_owner {
            group.owner = *id;
        }
        if m.verified
            && match group.verified_owner {
                Some(owner) if owner == previous_owner => replaces_owner,
                Some(owner) => earlier(*id, owner, &mut paths)?,
                None => true,
            }
        {
            group.verified_owner = Some(*id);
        }
    }
    for (id, m) in &s.tree.clones {
        let n = s.node(*id).ok_or("invalid clone node")?;
        let alias = n.hard_link && inodes.get(&(n.identity.device, n.identity.inode)) != Some(id);
        let group = groups.get(&(n.identity.device, m.clone_id));
        let deduction = if !alias && group.is_some_and(|g| g.consistent && g.owner != *id) {
            m.data_allocated_size
        } else {
            0
        };
        plan.clone_deduction_bytes = plan
            .clone_deduction_bytes
            .checked_add(deduction)
            .ok_or("clone deduction overflow")?;
        if m.verified {
            plan.clone_attributions.push((
                *id,
                (
                    !alias && group.is_some_and(|g| g.verified_owner == Some(*id)),
                    deduction,
                ),
            ));
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Content equality for snapshots: `Debug` of the whole tree would expose buffer capacities.
    fn content(s: &Snapshot) -> String {
        let nodes: Vec<_> = (0..s.node_count() as u32)
            .map(|id| s.legacy_node(id).unwrap())
            .collect();
        format!(
            "{nodes:?}{:?}{:?}{:?}{:?}{:?}",
            s.stats, s.accounting, s.errors, s.exclusions, s.state
        )
    }
    fn hardlink_fixture() -> Snapshot {
        let template = super::super::tests::fixture();
        let mut s = Snapshot::empty("/fixture".into());
        s.accounting.model = AccountingModel::FullClonesV1;
        let mut root = template.legacy_node(0).unwrap();
        root.children.clear();
        s.append_legacy_node(root).unwrap();
        // Owners replace repeatedly and across interleaved inode groups; the
        // resulting loser order is not dense-ID order until explicitly sorted.
        for (id, device, inode, path, linked) in [
            (1, 1, 100, "/fixture/z", true),
            (2, 1, 200, "/fixture/z-other", true),
            (3, 1, 100, "/fixture/y", true),
            (4, 1, 200, "/fixture/y-other", true),
            (5, 1, 100, "/fixture/x", true),
            (6, 1, 300, "/fixture/tie", true),
            (7, 1, 100, "/fixture/a", true),
            (8, 1, 200, "/fixture/a-other", true),
            (9, 1, 300, "/fixture/tie", true),
            (10, 2, 100, "/fixture/a-device", true),
            (11, 2, 100, "/fixture/z-device", true),
            (12, 1, 1200, "/fixture/ordinary", false),
        ] {
            let mut n = template.legacy_node(1).unwrap();
            n.id = id;
            n.parent = Some(0);
            n.children.clear();
            n.path = path.into();
            n.name = path.rsplit('/').next().unwrap().into();
            n.identity.device = device;
            n.identity.inode = inode;
            n.hard_link = linked;
            n.clone_metadata = None;
            s.append_legacy_node(n).unwrap();
        }
        s.set_children(0, &(1..13).collect::<Vec<_>>()).unwrap();
        s
    }

    fn empty_hardlink_plan() -> InternalAllocationPlan {
        InternalAllocationPlan {
            aliases: Vec::new(),
            clone_deduction_bytes: 0,
            clone_attributions: Vec::new(),
        }
    }

    /// Own sizes of the hard-link fixture after aliases are zeroed.
    const HARDLINK_VALUES: [u64; 13] = [0, 0, 0, 0, 0, 0, 120, 120, 120, 0, 120, 0, 120];
    type Owners = Vec<((u64, u64), u32)>;

    /// The elected owners (also the partial map at a first error), and on success the aliases and
    /// attributed values. The scratch buffers must grow like one-at-a-time insertion.
    fn elect_hardlinks(s: &Snapshot) -> (Result<(), String>, Owners, Vec<u32>, Vec<u64>) {
        let mut plan = empty_hardlink_plan();
        let mut owners = HashMap::new();
        let before: Vec<_> = (0..s.node_count())
            .map(|id| format!("{:?}", s.tree.records[id]))
            .collect();
        let got = collect_hardlinks(
            s,
            &mut plan,
            &mut owners,
            &mut [OwnerPathScratch::new(s), OwnerPathScratch::new(s)],
        );
        assert_eq!(
            before,
            (0..s.node_count())
                .map(|id| format!("{:?}", s.tree.records[id]))
                .collect::<Vec<_>>()
        );
        let mut elected: Owners = owners.iter().map(|(key, id)| (*key, *id)).collect();
        elected.sort();
        if got.is_err() {
            // Err drops the private plan, so its partial aliases are not part of the contract.
            return (got, elected, vec![], vec![]);
        }
        let mut reference = HashMap::new();
        for (key, id) in &elected {
            reference.insert(*key, *id);
        }
        assert_eq!(owners.capacity(), reference.capacity());
        let mut reference = Vec::new();
        for alias in &plan.aliases {
            reference.push(*alias);
        }
        assert_eq!(plan.aliases.capacity(), reference.capacity());
        let aliases = plan.aliases.clone();
        (got, elected, aliases, plan.materialize(s).values)
    }

    #[test]
    fn hardlink_owners_errors_and_scratch() {
        let owners: Owners = vec![((1, 100), 7), ((1, 200), 8), ((1, 300), 6), ((2, 100), 10)];
        let aliases = vec![1, 2, 3, 4, 5, 9, 11];
        let failed: Owners = vec![((1, 100), 5), ((1, 200), 4), ((1, 300), 6)];
        for variant in 0..7 {
            let mut s = hardlink_fixture();
            match variant {
                1 => {
                    for id in 1..13 {
                        s.update_node(id, |n| n.own_allocated_size = 0).unwrap();
                    }
                }
                2 => {
                    // Hostile persisted flags retain the old any-hard_link sweep,
                    // including directory, symlink and Other kinds.
                    for (id, kind) in [
                        (3, NodeKind::Directory),
                        (5, NodeKind::Symlink),
                        (7, NodeKind::Other),
                    ] {
                        s.update_node(id, |n| n.identity.kind = kind).unwrap();
                    }
                }
                3 => s.tree.records[7].parent = 7,
                4 | 5 => {
                    s.tree.paths.push((7, "/historical/exact/override".into()));
                    s.tree.paths.sort_by_key(|(id, _)| *id);
                    s.tree.records[7].parent = 7;
                    if variant == 5 {
                        s.tree.records[8].parent = 8;
                    }
                }
                6 => {
                    s.tree.records[3].parent = 3;
                    s.tree.records[7].parent = 7;
                }
                _ => {}
            }
            let invalid = Err("invalid path ancestry".to_string());
            let expected = match variant {
                1 => (Ok(()), owners.clone(), aliases.clone(), vec![0; 13]),
                3 | 5 => (invalid, failed.clone(), vec![], vec![]),
                4 => (
                    Ok(()),
                    vec![((1, 100), 5), ((1, 200), 8), ((1, 300), 6), ((2, 100), 10)],
                    vec![1, 2, 3, 4, 7, 9, 11],
                    vec![0, 0, 0, 0, 0, 120, 120, 0, 120, 0, 120, 0, 120],
                ),
                6 => (invalid, vec![((1, 100), 1), ((1, 200), 2)], vec![], vec![]),
                _ => (
                    Ok(()),
                    owners.clone(),
                    aliases.clone(),
                    HARDLINK_VALUES.to_vec(),
                ),
            };
            assert_eq!(elect_hardlinks(&s), expected, "variant {variant}");
        }
        let s = hardlink_fixture();
        let mut plan = empty_hardlink_plan();
        let mut owners = HashMap::new();
        collect_hardlinks(
            &s,
            &mut plan,
            &mut owners,
            &mut [OwnerPathScratch::new(&s), OwnerPathScratch::new(&s)],
        )
        .unwrap();
        assert_eq!(owners[&(1, 100)], 7);
        assert_eq!(owners[&(1, 200)], 8);
        assert_eq!(owners[&(1, 300)], 6); // Equal paths retain the first owner.
        assert_eq!(owners[&(2, 100)], 10);
        assert_eq!(plan.aliases, vec![1, 2, 3, 4, 5, 9, 11]);
    }

    #[test]
    fn complete_plans_with_sparse_claims() {
        type MaterializedPlan = (Vec<u64>, Vec<(u32, (bool, u64))>, u64);
        let template = super::super::tests::fixture();
        let metadata = template.tree.clones[0].1.clone();
        for variant in 0..4 {
            let mut s = hardlink_fixture();
            for id in [1, 2, 3, 4, 5, 7, 8] {
                s.tree.clones.push((id, metadata.clone()));
            }
            match variant {
                1 => s.tree.clones.clear(),
                2 => {
                    for (_, m) in &mut s.tree.clones {
                        m.verified = false;
                    }
                }
                3 => {
                    s.tree.clones.clear();
                    s.tree.records[7].parent = 7;
                }
                _ => {}
            }
            for clones in [false, true] {
                let actual: Result<MaterializedPlan, String> = allocations(&s, clones).map(|p| {
                    let p = p.materialize(&s);
                    (p.values, p.clone_attributions, p.clone_deduction_bytes)
                });
                // Only verified claims attribute, and only a consistent group deducts (id 8: 100).
                let expected = match (variant, clones) {
                    (3, _) => Err("invalid path ancestry".to_string()),
                    (0, true) => {
                        let mut values = HARDLINK_VALUES;
                        values[8] -= 100;
                        let claims = [(1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (7, 0), (8, 100)];
                        Ok((
                            values.to_vec(),
                            claims.map(|(id, d)| (id, (id == 7, d))).to_vec(),
                            100,
                        ))
                    }
                    _ => Ok((HARDLINK_VALUES.to_vec(), vec![], 0)),
                };
                assert_eq!(actual, expected, "variant={variant}, clones={clones}");
            }
            if variant < 3 {
                let mut actual = s.clone();
                let mut expected = s;
                for _ in 0..2 {
                    reference_aggregate(&mut expected);
                    crate::aggregate(&mut actual);
                    assert_eq!(content(&actual), content(&expected));
                }
            }
        }
    }
    #[test]
    fn alias_lookup_matches_map_across_threshold_and_exact_inode_keys() {
        let keys = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (u64::MAX, 0),
            (0, u64::MAX),
            (u64::MAX, u64::MAX),
            (2, 0),
        ];
        for capacity in [0, 64] {
            for len in 0..=keys.len() {
                let mut aliases = HashMap::with_capacity(capacity);
                for (index, key) in keys[..len].iter().enumerate() {
                    aliases.entry(*key).or_insert_with(|| index.to_string());
                    aliases.entry(*key).or_insert_with(|| "duplicate".into());
                }
                let lookup = AliasLookup::new(&aliases);
                assert_eq!(matches!(lookup, AliasLookup::Small(_)), len <= 4);
                for device in [0, 1, 2, 3, u64::MAX] {
                    for inode in [0, 1, 2, 3, u64::MAX] {
                        let key = (device, inode);
                        let expected = aliases.get(&key);
                        let actual = lookup.get(&key);
                        assert_eq!(actual, expected, "len={len}, key={key:?}");
                        if let (Some(actual), Some(expected)) = (actual, expected) {
                            assert!(std::ptr::eq(actual, expected));
                        }
                    }
                }
                for (index, key) in keys[..len].iter().enumerate() {
                    assert_eq!(lookup.get(key), Some(&index.to_string()));
                }
            }
        }
    }

    #[test]
    fn allocation_cursor_applies_aliases_and_claims_across_zero_claims_and_sparse_gaps() {
        for (aliases, attributions) in [
            (vec![], vec![]),
            (
                vec![0, 3, 8],
                vec![
                    (0, (false, 0)),
                    (1, (true, 0)),
                    (3, (false, 0)),
                    (5, (false, 7)),
                    (8, (false, 0)),
                    (9, (false, 2)),
                ],
            ),
        ] {
            let plan = InternalAllocationPlan {
                aliases,
                clone_attributions: attributions,
                clone_deduction_bytes: 9,
            };
            let mut expected = vec![20; 10];
            for id in &plan.aliases {
                expected[*id as usize] = 0;
            }
            for (id, (_, deduction)) in &plan.clone_attributions {
                expected[*id as usize] -= *deduction;
            }
            for _ in 0..2 {
                let mut cursor = AllocationCursor::new(&plan);
                let actual: Vec<_> = (0..10).map(|id| cursor.value(id, 20)).collect();
                assert_eq!(actual, expected);
                assert_eq!(cursor.alias, plan.aliases.len());
                assert_eq!(cursor.attribution, plan.clone_attributions.len());
            }
        }
    }

    // Keep the former general-metadata update path as an independent oracle for
    // the size-only writes, including the compatibility allocation strategy.
    fn reference_aggregate(s: &mut Snapshot) {
        let plan = internal_allocation_plan(s).unwrap();
        s.accounting.deduction_bytes = plan.clone_deduction_bytes;
        let mut cursor = AllocationCursor::new(&plan);
        for id in 0..s.node_count() {
            s.update_node(id as u32, |n| {
                n.allocated_size = cursor.value(id as u32, n.own_allocated_size);
            })
            .unwrap();
        }
        s.tree.clone_attributions = plan.clone_attributions;
        for id in (0..s.node_count()).rev() {
            let n = s.node(id as u32).unwrap();
            if let Some(parent) = n.parent {
                let allocated = n.allocated_size;
                s.update_node(parent, |n| {
                    n.allocated_size = n.allocated_size.saturating_add(allocated);
                })
                .unwrap();
            }
        }
        if let Some(root) = s.node(0) {
            s.stats.allocated_bytes = root.allocated_size;
        }
    }

    #[test]
    fn compact_aggregation_matches_general_updates_across_chunks_and_accounting_modes() {
        let template = super::super::tests::fixture();
        for variant in 0..4 {
            let mut actual = template.clone();
            for id in 4..8201 {
                let mut node = template.legacy_node(2).unwrap();
                node.id = id;
                // Nested parents span both record chunk boundaries.
                let parent = if id <= 4096 { 0 } else { 4096 };
                node.parent = Some(parent);
                node.path = if parent == 0 {
                    format!("/fixture/n{id}").into()
                } else {
                    format!("/fixture/n4096/n{id}").into()
                };
                node.name = format!("n{id}");
                node.identity.inode = u64::from(id) + 10;
                node.is_directory = id == 4096;
                node.identity.kind = if node.is_directory {
                    NodeKind::Directory
                } else {
                    NodeKind::File
                };
                node.clone_metadata = None;
                node.excluded = id % 7 == 0;
                node.length = if id == 8200 { u64::MAX } else { u64::from(id) };
                node.own_allocated_size = node.length;
                actual.append_legacy_node(node).unwrap();
            }
            actual
                .set_children(4096, &(4097..8201).collect::<Vec<_>>())
                .unwrap();
            actual
                .set_children(0, &(1..4097).collect::<Vec<_>>())
                .unwrap();
            match variant {
                1 => actual.tree.clones[1].1.verified = false,
                2 => {
                    actual.tree.clones.clear();
                    actual.accounting.model = AccountingModel::HardLinksV1;
                }
                3 => actual.tree.clones[1].1.data_length = 99,
                _ => {}
            }
            let mut expected = actual.clone();
            for _ in 0..2 {
                reference_aggregate(&mut expected);
                crate::aggregate(&mut actual);
                // Includes names, topology, identities, raw sizes, flags, clone
                // claims/attributions, statistics, and accounting diagnostics.
                assert_eq!(
                    content(&actual),
                    content(&expected),
                    "accounting variant {variant}"
                );
                assert_eq!(actual.stats.allocated_bytes, u64::MAX);
            }
        }
        let mut empty = Snapshot::empty("/empty".into());
        let mut expected = empty.clone();
        reference_aggregate(&mut expected);
        crate::aggregate(&mut empty);
        assert_eq!(content(&empty), content(&expected));
    }

    #[test]
    fn repeated_aggregation_is_stable() {
        for variant in 0..5 {
            let mut s = super::super::tests::fixture();
            match variant {
                1 => s.tree.clones[1].1.verified = false,
                2 => {
                    for (_, metadata) in &mut s.tree.clones {
                        metadata.data_allocated_size = 0;
                    }
                }
                3 => {
                    s.tree.clones.clear();
                    s.accounting.model = AccountingModel::HardLinksV1;
                }
                4 => s.tree.clones[1].1.data_length = 99,
                _ => {}
            }
            {
                crate::aggregate(&mut s);
                let values: Vec<_> = s.iter_nodes().map(|n| n.allocated_size).collect();
                crate::aggregate(&mut s);
                assert_eq!(
                    values,
                    s.iter_nodes().map(|n| n.allocated_size).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn nonshared_large_plan_does_not_allocate_dense_values() {
        let template = super::super::tests::fixture();
        let mut s = Snapshot::empty("/fixture".into());
        for id in 0..100_001 {
            let mut node = template.legacy_node(if id == 0 { 0 } else { 1 }).unwrap();
            node.id = id;
            node.parent = (id != 0).then_some(0);
            node.path = if id == 0 {
                "/fixture".into()
            } else {
                format!("/fixture/{id}").into()
            };
            node.name = id.to_string();
            node.children.clear();
            node.hard_link = false;
            node.clone_metadata = None;
            s.append_legacy_node(node).unwrap();
        }
        let sparse = internal_allocation_plan(&s).unwrap();
        assert!(sparse.aliases.is_empty());
    }

    #[test]
    fn claim_cursor_matches_binary_search_for_sparse_claims() {
        let metadata = super::super::tests::fixture().tree.clones[0].1.clone();
        for ids in [vec![], vec![1, 4, 9]] {
            let claims: Vec<_> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let mut m = metadata.clone();
                    m.clone_id = i as u64 + 1;
                    (*id, m)
                })
                .collect();
            let original = |id| {
                claims
                    .binary_search_by_key(&id, |(id, _)| *id)
                    .ok()
                    .map(|i| &claims[i].1)
            };
            let mut cursor = ClaimCursor::new(&claims);
            for id in 0..12 {
                assert_eq!(cursor.get(id), original(id));
            }
            for (index, (id, _)) in claims.iter().enumerate() {
                assert_eq!(cursor.at(index), original(*id));
            }
        }
    }

    fn assert_path_order_oracle(s: &Snapshot) {
        let mut paths = [OwnerPathScratch::new(s), OwnerPathScratch::new(s)];
        let ids: Vec<_> = (0..s.node_count() as u32).chain([u32::MAX]).collect();
        for reverse in [false, true] {
            let mut traversal = ids.clone();
            if reverse {
                traversal.reverse();
            }
            for candidate in &traversal {
                for owner in &ids {
                    let expected = if candidate == owner {
                        Ok(false)
                    } else {
                        s.path(*candidate)
                            .and_then(|candidate| s.path(*owner).map(|owner| candidate < owner))
                    };
                    assert_eq!(earlier(*candidate, *owner, &mut paths), expected);
                }
            }
        }
    }

    #[test]
    fn cached_owner_paths_match_original_order_and_historical_errors() {
        let template = super::super::tests::fixture();
        let mut s = Snapshot::empty("/fixture".into());
        let mut root = template.legacy_node(0).unwrap();
        root.children = vec![];
        s.append_legacy_node(root).unwrap();
        for (id, parent, path, directory) in [
            (1, 0, "/fixture/a", true),
            (2, 0, "/fixture/a-1", true),
            (3, 1, "/fixture/a/z", false),
            (4, 2, "/fixture/a-1/x", false),
            (5, 1, "/historical/exact/override", false),
            (6, 2, "/fixture/a-1/..", false),
        ] {
            let mut node = template.legacy_node(1).unwrap();
            node.id = id;
            node.parent = Some(parent);
            node.path = path.into();
            node.name = path.rsplit('/').next().unwrap().into();
            node.is_directory = directory;
            node.hard_link = false;
            node.identity.kind = if directory {
                NodeKind::Directory
            } else {
                NodeKind::File
            };
            node.children.clear();
            s.append_legacy_node(node).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let mut node = template.legacy_node(1).unwrap();
            node.id = 7;
            node.parent = Some(1);
            node.path = std::ffi::OsString::from_vec(b"/fixture/a/raw-\xff".to_vec()).into();
            node.name = "raw-�".into();
            node.children.clear();
            s.append_legacy_node(node).unwrap();
        }
        assert_path_order_oracle(&s);
        // Noncanonical legacy names use the old byte-preserving reconstruction.
        let name_offset = s.tree.records[3].name_offset as usize;
        s.tree.names[name_offset] = b'.';
        assert_path_order_oracle(&s);
        // The legacy override remains authoritative even with invalid ancestry;
        // a non-overridden invalid node must retain the old error text.
        s.tree.records[5].parent = 5;
        assert_path_order_oracle(&s);
        s.tree.records[3].parent = 3;
        assert_path_order_oracle(&s);
        s.tree.paths.insert(0, (0, "/historical/root".into()));
        s.tree.records[0].parent = 0;
        assert_path_order_oracle(&s);
    }

    #[test]
    fn plan_matches_separate_accounting_and_exposes_only_clone_deduction() {
        let s = super::super::tests::fixture();
        let p = allocation_plan(&s).unwrap();
        assert_eq!(p.values, vec![0, 0, 140, 20]);
        assert_eq!(p.clone_deduction_bytes, 100);
        assert_eq!(
            p.clone_attributions,
            vec![(1, (false, 0)), (2, (true, 0)), (3, (false, 100))]
        );
    }

    #[test]
    fn ownership_uses_paths_independently_of_node_order() {
        // Permute native spellings, preserving the identities and hard-link
        // aliases. Every ordering must elect the path minimum in both stages.
        for names in [
            ["z", "a", "alias"],
            ["a", "z", "alias"],
            ["alias", "z", "a"],
            ["z", "alias", "a"],
            ["a", "alias", "z"],
            ["alias", "a", "z"],
        ] {
            let mut s = super::super::tests::fixture();
            for (i, name) in names.iter().enumerate() {
                let id = i as u32 + 1;
                let mut n = s.legacy_node(id).unwrap();
                n.name = (*name).into();
                n.path = format!("/fixture/{name}").into();
                s.replace_node(id, n).unwrap();
            }
            let p = allocation_plan(&s).unwrap();
            let inode_owner = if names[0] < names[2] { 1 } else { 3 };
            let clone_owner = if names[inode_owner - 1] < names[1] {
                inode_owner
            } else {
                2
            };
            let mut expected = vec![0, 120, 140, 120];
            expected[if inode_owner == 1 { 3 } else { 1 }] = 0;
            expected[if clone_owner == 2 { inode_owner } else { 2 }] -= 100;
            assert_eq!(p.values, expected);
            assert_eq!(p.clone_deduction_bytes, 100);
            assert_eq!(
                p.clone_attributions
                    .iter()
                    .filter(|(_, (owner, _))| *owner)
                    .count(),
                1
            );
            assert_eq!(p.clone_attributions[clone_owner - 1].1, (true, 0));
        }
    }

    #[test]
    fn inconsistent_and_unverified_groups_never_report_deductions() {
        for unverified in [false, true] {
            let mut s = super::super::tests::fixture();
            if unverified {
                s.tree.clones[1].1.verified = false;
            } else {
                s.tree.clones[1].1.data_length = 99;
            }
            let p = allocation_plan(&s).unwrap();
            assert_eq!(p.values, vec![0, 0, 140, 120]);
            assert_eq!(p.clone_deduction_bytes, 0);
            assert!(p
                .clone_attributions
                .iter()
                .all(|(_, (_, deduction))| *deduction == 0));
        }
    }
}
