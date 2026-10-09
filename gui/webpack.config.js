// Builds TurboWarp's scratch-gui editor with `scratch-vm` and `scratch-render`
// resolved to the scrust packages in ./packages. Run from vendor/scratch-gui
// (see scripts/build.js) so that the GUI's own relative paths resolve.

const path = require('path');

const GUI = path.resolve(__dirname, 'vendor', 'scratch-gui');
const guiRequire = name => require(path.join(GUI, 'node_modules', name));
const HtmlWebpackPlugin = guiRequire('html-webpack-plugin');

// The first configuration is the playground (editor, player, ...).
const base = require(path.join(GUI, 'webpack.config.js'))[0];

module.exports = Object.assign({}, base, {
    context: GUI,
    entry: {
        'scrust-editor': path.resolve(__dirname, 'src', 'entry.js')
    },
    output: Object.assign({}, base.output, {
        path: path.resolve(__dirname, 'build')
    }),
    resolve: Object.assign({}, base.resolve, {
        alias: Object.assign({}, base.resolve.alias, {
            'scratch-vm$': path.resolve(__dirname, 'packages', 'scrust-vm', 'index.js'),
            'scratch-render$': path.resolve(__dirname, 'packages', 'scrust-render', 'index.js')
        }),
        // The packages live outside the GUI and use its dependencies (after
        // the usual hierarchical lookup, which keeps nested versions intact).
        modules: ['node_modules', path.join(GUI, 'node_modules')]
    }),
    resolveLoader: {
        modules: ['node_modules', path.join(GUI, 'node_modules')]
    },
    plugins: base.plugins
        .filter(p => !(p instanceof HtmlWebpackPlugin))
        .concat([
            new HtmlWebpackPlugin({
                chunks: ['scrust-editor'],
                template: 'src/playground/index.ejs',
                filename: 'editor.html',
                title: 'scrust - Scratch to Rust',
                isEditor: true,
                root: '',
                meta: {},
                APP_NAME: 'scrust'
            })
        ])
});
