// Bundles the reference page: a TurboWarp scratch-vm checkout (SCRUST_REF_VM)
// with TurboWarp's scratch-render, using the vendored scratch-gui's packages
// and loaders.  node ../../gui/vendor/scratch-gui/node_modules/.bin/webpack --config webpack.config.js
const path = require('path');
const GUI = path.resolve(__dirname, '..', '..', 'gui', 'vendor', 'scratch-gui');
const webpack = require(path.join(GUI, 'node_modules', 'webpack'));
const VM = process.env.SCRUST_REF_VM || '/Users/kobajin/repo/scratch-vm-develop';

module.exports = {
    mode: 'development',
    devtool: false,
    context: __dirname,
    entry: {twref: './entry.js'},
    output: {path: path.resolve(__dirname, '..', 'out', 'twref'), filename: '[name].js'},
    resolve: {
        symlinks: false,
        alias: {
            'scratch-vm$': path.join(VM, 'src', 'index.js'),
            'text-encoding$': path.join(GUI, 'src', 'lib', 'tw-text-encoder')
        },
        modules: ['node_modules', path.join(GUI, 'node_modules'), path.join(VM, 'node_modules')]
    },
    resolveLoader: {modules: [path.join(GUI, 'node_modules')]},
    // webpack 4's parser predates `??` / `?.`, which TurboWarp uses.
    module: {
        rules: [{
            test: /\.js$/,
            include: [path.join(VM, 'src'), /node_modules[\\/](scratch-|@turbowarp)/],
            loader: 'babel-loader',
            options: {
                babelrc: false,
                configFile: false,
                presets: [[require.resolve(path.join(GUI, 'node_modules', '@babel/preset-env')), {targets: 'chrome 70', modules: false}]]
            }
        }, {
            test: /\.mp3$/,
            loader: 'file-loader',
            options: {outputPath: 'media/music/'}
        }]
    },
    plugins: [
        new webpack.NormalModuleReplacementPlugin(/[\\/]util[\\/]uid(\.js)?$/, path.join(__dirname, 'uid.js'))
    ],
    performance: {hints: false}
};
