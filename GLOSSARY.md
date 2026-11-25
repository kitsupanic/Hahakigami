# Hahakigami

Disk-space measurements and retained results for local storage analysis.

## Language

**Edition**:
One of the two ways the app is shipped from the same source: the Full edition and the Lite edition. Each is a separate app with its own name, so both can be installed at once and they share no saved state.
_Avoid_: Variant, flavour, build type, SKU

**Full edition**:
The edition shipped as Hahakigami, downloaded directly rather than from the Mac App Store. It has every feature, including Automation scans.
_Avoid_: Pro, direct build, GitHub version

**Lite edition**:
The edition shipped as Hahakigami Lite on the Mac App Store. It lacks only what Apple's App Sandbox forbids; nothing is held back to push people to the Full edition, and it never mentions the Full edition.
_Avoid_: Free version, App Store build, sandboxed build, demo

**Allowed location**:
A disk or folder the person has picked in an Open dialog so the Lite edition may read it, remembered across launches. In the Lite edition every Scan target is an Allowed location or lies inside one. Allowing the startup disk covers every disk, since other disks are mounted inside it. Full Disk Access still widens what a scan inside it can read.
_Avoid_: Grant, bookmark, authorized folder, permitted folder

**Automation scan**:
A filesystem-metadata measurement requested by an agent or script, independently of the desktop explorer's Disk scans.
_Avoid_: Desktop session, remote scan

**Full Disk Access**:
The macOS privacy setting, in System Settings → Privacy & Security, that lets an app read protected user data without per-folder prompts. macOS grants it and enforces it; the user turns it on or off.
_Avoid_: File access, disk permission, grant

**Full Disk Access status**:
What the app can tell about Full Disk Access for the running copy: on, off, or unknown. Unknown means the app could not tell, never that access is off.
_Avoid_: Acknowledgement, confirmation, setup done

**Disk scan**:
The latest measurement of one disk made in this session. Every disk in the Disks list keeps its own, so scanning a second disk never discards the first one's results, and disks can be scanned at the same time. A folder scan is kept the same way, but all folders share one. It stays available until the next scan of that disk (or of any folder) replaces it or the disk is ejected. Its results are gone once the app quits. Network shares are not supported.
_Avoid_: Current scan, live scan

**Scan slot**:
Where the app keeps a Disk scan while the session lasts. Every disk has its own, and all folder scans share one more. A scan can start only while its own slot is not scanning; starting it replaces that slot's previous scan and no other.
_Avoid_: Session slot, scan cache

**Scan all disks**:
The action that starts a fresh Disk scan of every disk in the Disks list. A disk that is already scanning is cancelled and scanned again. Folder scans are not touched. In the Lite edition it covers only disks that are Allowed locations and says how many it skipped.
_Avoid_: Scan everything, rescan all

**Scan budget**:
The workers and open files the whole app may use for scanning. Scans that run at the same time split it, so several disks can be scanned at once without starving the Mac. A scan gets its share when it starts and gives it back when it ends.
_Avoid_: Scan limit, thread pool

**Rescan baseline**:
The earlier completed scan of the same location that a new scan starts from, so only what changed since is measured again. Without one, the scan measures everything.
_Avoid_: Seed, cached scan, previous scan

**On disk**:
The space a file or folder occupies in allocated blocks, as recorded by the scan. It is the only size the app shows. Verified scans deduct shared APFS clone blocks. It does not predict space freed by moving items to the Trash.
_Avoid_: Scanned on disk, on disk in scan, recorded allocation, allocated size

**Length**:
A file's byte count. The scanner keeps it only to verify clones and to detect changed files on a rescan. It is never shown or summed per folder.
_Avoid_: Real size, apparent size, content size

**Scan warning**:
A note that some sizes in a scan may be slightly off because files changed while they were being measured. The rest of the scan is accurate.
_Avoid_: Accounting warning, validation error

**Scan target**:
The disk or folder the next scan will measure. It is chosen from the Disks list in the sidebar, the sidebar's Scan folder button, or the welcome page's folder picker, and may differ from the location of the scan on screen.
_Avoid_: Next scan, scan picker

**Cleanup queue**:
Items the person has picked to move to the Trash later, kept across launches. Each entry remembers the item, its folder chain and its volume as they were when it was added, and is moved to the Trash only after a scan confirms nothing has changed since.
_Avoid_: Trash list, cart, basket

**Matched scan**:
The one Disk scan or folder scan a Cleanup queue entry is checked against: its Own root scan if there is a usable one, otherwise its Covering disk. An entry with no matched scan needs a scan before it can be cleaned, and it is cleaned only through its matched scan.
_Avoid_: Owning scan, source scan

**Own root**:
The disk or folder whose scan an entry was added from. A completed scan of that same location checks the entry most strictly, and can report the item missing or changed.
_Avoid_: Origin scan, queue root

**Covering disk**:
A completed Disk scan of a disk that contains an entry's item, used when there is no scan of the entry's Own root. Of several, the one with the deepest root wins. A folder scan never covers another location. If the covering disk disagrees with the entry, the entry simply needs a scan; the disk is never taken as proof that the item changed.
_Avoid_: Parent scan, fallback scan
