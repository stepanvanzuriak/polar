# native_bridge_client

`Posts in Native from Node`: a native program calls effects that live on a Node server.

```sh
polar run                       # JavaScript on Node: Posts is called locally
polar run --target native       # native: builds the Node bridge server, starts it
                                # as a sub-service, runs the executable against it,
                                # then stops the server
```

`polar run --target native` finds the bridge from the source: `Native` is the client host and `Node`
the server host, so it builds the Node bundle, serves `_polar/bridges.js` on a free port and
sets `POLAR_BRIDGE` for the executable. A built executable (`polar build --target native`) reads
`POLAR_BRIDGE` (default `http://127.0.0.1:3000/_polar`) to find the server.

Plain `http://` only; TLS is not supported by the native runtime.
