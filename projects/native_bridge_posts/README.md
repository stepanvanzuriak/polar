# native_bridge_posts

The server half of a bridge, running as a native executable.

`Posts in Browser from Node` forwards `Posts` from a browser to the Node host over HTTP. The native
target is a Node-family host, so there is no browser to forward from and no HTTP listener (Http and
bridges on the native host are out of scope for P26). What does work natively is the `Node` side:

```sh
polar run --target native                  # runs `main`, calling Posts locally
polar build --target native                # → dist/native_bridge_posts
polar build --host Browser                 # the same source, bridged client for the web
```

The bridge declaration stays in the source and is simply not part of a native build, so one project
serves the web client (JavaScript build) and a CLI (native build).
