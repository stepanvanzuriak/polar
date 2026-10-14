# native_bridge_server

`Posts in Browser from Native`: a browser (or any JavaScript) client calls effects that live in a
native executable.

```sh
polar build --target native            # → dist/native_bridge_server
./dist/native_bridge_server --serve 3000
polar build --host Browser             # the bridged client bundle for the web
```

`--serve <port>` makes the executable answer `POST /_polar/<Effect>/<op>` on `127.0.0.1:<port>` with
the same wire format as the Node bridge server (`{"args": […]}` in, `{"ok": …}` or
`{"throw": {"tag", "value"}}` out), so browser bundles need no change: point them at it with
`bridge.configure({ base: "http://127.0.0.1:3000/_polar" })`. Without `--serve` the program runs `main`.
