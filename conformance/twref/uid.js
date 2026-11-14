// scratch-vm draws its internal ids from Math.random; a counter keeps the
// seeded sequence for the project (see harness/trace.js).
let n = 0;
module.exports = () => `uid-${n++}`;
