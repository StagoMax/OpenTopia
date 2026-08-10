# Archive extraction can escape its destination

`buildExtractionPlan(root, entries)` is used before archive contents are
written. It currently relies on string prefixes and accepts traversal, absolute
Windows paths, and unsafe symbolic-link targets.

Return normalized plan rows `{ path, type, destination, target }`, sorted by
archive path. File and directory entries use `target: null`. Symlink targets are
stored exactly as normalized safe relative paths.

Reject:

- absolute, UNC, drive-prefixed, empty, `.` or `..` archive paths;
- paths containing empty segments after converting `\\` to `/`;
- duplicate normalized paths;
- symlink targets that are absolute or escape the symlink's parent;
- entry types other than `file`, `directory`, and `symlink`.

The function must not touch the filesystem or mutate inputs.
