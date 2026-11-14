# AGENTS.md

Working rules for agents (Claude Code and others) operating in this repository.

## Commit policy

- **Commit directly to `main` whenever you reach a sensible stopping point.** No confirmation needed.
  - Examples of a sensible stopping point: one feature or fix is finished, a milestone in PRODUCT.md §18 is done, an acceptance-criteria improvement is in place, or the docs have been updated.
  - Don't mix unrelated changes in one commit; split them into units of meaning.
- Check these before committing:
  - `npx tsc --noEmit` passes
  - `npm test` passes
  - If you changed the renderer, at least run `npm run accept -- --only A1` (determinism). When the change is large, also run `npm run accept -- --quick`.
- Commit messages: a short summary in English or Japanese on the first line; put the background and verification results in the body when needed.
- Never commit generated files or large binaries (`out/`, `dist/`, `examples/sample/`, `.collider/`, `.venv/`, `node_modules/`, `models/`). They are already in `.gitignore`.
- Push or open a PR only when the user asks you to.

## Project essentials

- The spec is [PRODUCT.md](PRODUCT.md); usage is in [README.md](README.md).
- `src/core/` is isomorphic (Node and browser). Using `Math.random` is forbidden; use the functional RNG in `src/core/rng.ts` (PRODUCT.md §14.2).
- Don't edit `src/` while a render or the acceptance tests are running. `render` rebuilds `dist/` from the latest sources, so a mid-run edit breaks the A1 comparison.
- Python (analysis and ingest) uses `.venv/` (librosa etc.). You can override the interpreter with `COLLIDER_PYTHON`.
