# Local renderer dependency

Three.js 0.186.0, MIT license (THREE-LICENSE.txt), copied from the pinned
`tools/asset-import/node_modules/three` package. Browser assets require no CDN.

- build/three.module.js → three.module.js
- build/three.core.js → three.core.js
- examples/jsm/controls/OrbitControls.js → OrbitControls.js

Only patch: OrbitControls imports `./three.module.js` instead of bare `three`.
Reproduce from `npm ci --ignore-scripts` in tools/asset-import. Source:
https://github.com/mrdoob/three.js/tree/r186

Imported printer geometry retains its own source terms; this renderer license does
not relicense those assets or the application.
