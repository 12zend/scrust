# Unicode version differences

`toLowerCase`/`toUpperCase` use Rust's standard library tables (default case mapping, SpecialCasing, final sigma), which may be a different Unicode version than the user's browser. `trim()` and `Number()` use the ECMAScript WhiteSpace + LineTerminator set as of Unicode 15 (U+180E is not whitespace). Differences can only show up for characters whose case mapping changed between versions.
