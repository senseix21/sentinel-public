# Vendored browser assets

## `livekit-client.umd.js`

- Package: `livekit-client`
- Version: 2.21.0
- License: Apache-2.0
- SHA-256: `a77a2f4c363e93099d7c135721c9ec81d6c5bacc691796dad799222e33cbfb31`
- Size: 561,757 bytes
- Obtained with: `npm pack livekit-client@2.21.0`, then
  `tar xzf livekit-client-2.21.0.tgz package/dist/livekit-client.umd.js`

This file is committed rather than fetched because the dashboard must make no
external request at runtime, and the repository must have no frontend build
step. It is `include_str!`d into the server binary exactly like
`ui/dashboard.html` and served from `/ui/livekit-client.js`, same-origin.

It is a prebuilt UMD bundle taken unmodified from the published package. Do not
edit it. To upgrade, repeat the two commands above with the new version and
update this file's version, hash, and size.

See `docs/adr/010-livekit-browser-client-in-the-dashboard.md` for why a live
player was added and why the SDK is vendored instead of loaded from a CDN.
