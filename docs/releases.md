# Release preparation

Cargo.toml is the version source for the application and packaging scripts. Do not edit version strings in Rust code or package scripts.

From the source directory:

```sh
cargo run --locked --features release-tools --bin release-tool -- bump 0.4.2
```

Add `--dry-run` to validate without writing. The tool updates the package version, its Cargo.lock entry, the README introduction, and a new changelog section. Dependency versions and older changelog entries stay intact. Replace the new changelog placeholder with the changes users will receive.

Validate the result:

```sh
cargo run --locked --features release-tools --bin release-tool -- check
cargo fmt --check
cargo test --locked --features release-tools --bin release-tool
```

CI checks these references on pull requests. Stable three-part versions must fit Windows MSI limits: major and minor up to 255, patch up to 65535. Start with a clean working tree so the changes are easy to review.

Commit on a branch and open a pull request. After checks pass and the PR is merged, build packages from that commit, verify their versions and checksums, then create the matching vVERSION tag and GitHub release. Publishing requires separate approval. This tool never pushes, tags, uploads, or publishes.

The release-tools feature is only for development; normal application and package builds exclude the tool.
