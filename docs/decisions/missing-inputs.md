# Empty input slots (`undefined`)

An empty boolean slot has no entry in the block's inputs, so scratch-vm passes `undefined`. In editor-made projects that value only ever reaches `Cast.toBoolean` (`if <>`, `not <>`, `wait until <>` ...), which yields `false`.

scrust lowers an empty slot to `Expr::Missing`: `false` as a boolean, `0` as a number, `"undefined"` as a string. Storing it into a variable stores the string `"undefined"` (scratch-vm would store the JS `undefined`); that can only happen in hand-edited projects and is the one known deviation.

Empty *procedure* arguments are different: the call uses the prototype's `argumentdefaults` (`""` for text, the **string** `"false"` for booleans).
