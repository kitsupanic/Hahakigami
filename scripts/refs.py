"""Single source of truth for the pinned upstream comparator commits."""
REFS = {"Radix": "df1235bfb05acb7a678718166491bcc27817d95d", "OpenDisk": "f7551cf21a5c35a963c607401ba5705ec0869b64"}


def write_swift_ref(directory, name):
    """Emit the pinned ref for the Swift adapters, which compile it in via this file."""
    path = directory / f"{name}SourceRef.swift"
    path.write_text(f'nonisolated let benchmarkSourceRef = "{REFS[name]}"\n')
    return path
