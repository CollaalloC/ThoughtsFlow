# Recognized development migration lineage

`0005_context_tree_development.sql` is an exact frozen reconstruction of the
pre-commit Context Tree migration applied during local development on 2026-07-29.
It is **not** part of the numbered migration directory and is never applied to
new databases. Its SHA-384 is:

```text
bbaa28ed3b82d0c72c738bd2d56048a59667ae69dccb29cbccde062ba9c2f6a4f92702de73a11e8b0c3903b52aa52921
```

The 16,328-byte script equals canonical migration 5 at commit
`2779d7cf273245fb048927e7caea8b65aabed997`, except for the two later additions:
`context_manifest_item_typed_source_identity` and
`ux_context_checkpoint_workspace_created_at`, including their adjacent comments
and trailing blank lines. The original script's checksum and schema were both
verified independently. No user database or user content is distributed here.

The repository selects this lineage only for that exact recorded checksum. It
then verifies every applied migration against a contiguous known history and
compares all application schema objects with an empty database built from the
same known lineage. Unknown checksums and schema modifications remain errors.

SQLx still validates and applies migrations. Migration 9 adds the two missing
constraints, and a SQLx-managed outer `BEGIN IMMEDIATE` transaction covers
validation and the whole remaining development-database upgrade. If a unique
checkpoint timestamp constraint fails, all changes roll back, including the
mutable default-provider marker from migration 8. Checkpoint times, historical
receipts, request bytes, hashes, and old migration metadata are never rewritten.
The same lineage remains recognized after a successful upgrade and restart.

Keep this SQL file byte-for-byte stable and LF-terminated on every platform.
Do not replace canonical migration 5 or overwrite stored migration checksums.
