## GitHub Copilot Agent Guidelines

This repository uses AI coding agents (like GitHub Copilot) to help implement features and refactors. These guidelines describe how agents should behave when modifying this codebase.

### Scope and Goals

- Assist with implementing exercises and features in `mini-lsm-starter`, following the project’s README and book.
- Prefer minimal, surgical changes that solve the specific task without broad refactors.
- Keep the main `mini-lsm` and `mini-lsm-mvcc` crates as references; do not modify them unless explicitly asked.

### General Rules

- Do not run `git commit`, create branches, or change project licensing.
- Follow existing code style and patterns in this repository (Rustfmt config, naming, error handling, etc.).
- When in doubt about behavior, check the reference implementations under `mini-lsm` and the book in `mini-lsm-book`.
- Never paste large amounts of external code; write original code tailored to this repo.

### Working in `mini-lsm-starter`

- Treat `mini-lsm-starter` as the primary workspace for implementing missing pieces.
- Use the tests in `mini-lsm-starter/src/tests.rs` and per-module bins as guidance and validation.
- Do not "optimize away" steps that the tutorial/book intends the user to learn, unless the user explicitly requests it.

### Tests and Tooling

- Prefer running focused tests (e.g. `cargo test --package mini-lsm-starter --lib -- tests::week1_day4::test_sst_seek_key --exact --nocapture`) for the area you change.
- If compilation or tests fail due to your changes, fix the root cause in the starter crate.
- Do not attempt to fix unrelated failing tests or lints unless the user requests.

### Documentation and Comments

- Keep comments short and practical; avoid explaining obvious Rust features.
- Update README or book markdown only if the user asks for documentation help.

### Interaction with the User

- Ask for clarification only when necessary; otherwise, implement the most reasonable interpretation of the request.
- Summarize what you changed and where, and suggest next steps (e.g., which tests to run).

### Debugging

- User work mostly under `mini-lsm-starter`, reference are provided in `mini-lsm`
- When helping the user, state clearly what does the user do wrong, and what the functionality should be.
- When the user get stuck on tests, compare user's implementation with the ones of the reference, give hints and guidance to help the user catch up with the reference implementation.
- The reference is not a must. If the user's implementation could be changed to meet the tests and functional requirements, help the user to do it right, reminding the gap between current code and ideal code.