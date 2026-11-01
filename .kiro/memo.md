# Starlinkmon 启动方式

```bash
cd /Volumes/z/starlinkmon/src-tauri && cargo tauri build --debug
open /Volumes/z/starlinkmon/src-tauri/target/debug/bundle/macos/starlinkmon.app
```

- 不能用 `cargo run`：Tauri 不会嵌入前端资源，导致白屏
- 不能用 `cargo tauri dev`：项目没有前端 dev server，dist/ 是预编译好的静态文件
