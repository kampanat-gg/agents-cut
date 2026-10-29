# Working on this repository

The Cargo workspace is `src/`, not the repository root: run every `cargo`
command from there.

## Building on this machine

Development happens on a Mac mini M4 with 16 GB of RAM, 10 cores and a
228 GB disk. The `concat` crate's Slint-generated module is huge. One rustc
of it takes several gigabytes, and every change of feature or profile leaves
another multi-gigabyte incremental cache behind. Running `cargo test` and
`cargo clippy` back to back once crashed the machine, with 63 GB of stale
caches in `target/quick/incremental`.

- Run one cargo command at a time, never two in parallel or in the
  background alongside another, and always with `-j 4`.
- While iterating, use `cargo check -p concat --profile quick -j 4`. Run
  the tests and clippy once, one after the other, just before a commit.
- Build only the crates you touched (`-p concat`), not the whole
  workspace.
- Do not build or relaunch the test app while the user's in-app agent is
  running a turn.
- Check `du -sh src/target/quick/incremental` before a long session and
  after changing features or profiles. When it passes about 20 GB, or the
  disk has less than 30 GB free, ask the user before deleting it. It is
  safe to delete: the next build is just slower.
- Keep scratch builds, frames and app bundles out of the repository. Clear
  your own scratch output when a task is done.
