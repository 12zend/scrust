# project.json details

- Object key order follows JavaScript: canonical array indices first in ascending order, then insertion order. Applied to blocks (script order), inputs (evaluation order), variables, lists and broadcasts.
- JSON numbers stay numbers and strings stay strings, including inside literal primitives (`[4, "10"]` is the string `"10"`).
- Lone surrogate escapes (`"\ud83d"`) are valid in JS strings but rejected by `serde_json`; they are carried through as supplementary private-use characters and restored when building `JsStr`.
- `next` cycles are cut; dangling block ids evaluate as empty inputs.
