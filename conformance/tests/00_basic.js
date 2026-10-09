const {Project, b} = require('../harness/sb3');

module.exports = () => {
    const cases = [];
    {
        // PRODUCT.md §9.2 example.
        const p = new Project();
        const s = p.sprite('Sprite1');
        const i = p.stage.v('i', 0);
        s.script(b.whenFlag(), [b.set(i, 0), b.repeat(10, [b.change(i, 1), b.say(b.join('i=', i))])]);
        cases.push({name: 'counter', project: p, frames: 15});
    }
    return cases;
};
