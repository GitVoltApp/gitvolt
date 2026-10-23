# Upstream report draft: tauri-bundler can't bundle a .deb or .rpm with CEF 154

Not filed; filing is the user's call. GitVolt works around it with `scripts/cef-bundler-dist.sh`
(`just package`).

---

Target: `tauri-apps/tauri` (`crates/tauri-bundler`, branch `v3`).

**Title:** [bug] tauri-bundler (v3 alpha): `.deb`/`.rpm` bundling fails with CEF 154: libEGL.so and libGLESv2.so no longer exist

### Describe the bug

`tauri-runtime-cef` 3.0.0-alpha.6 moved to CEF 154 (`cef` 154.5.0+154.0.34, Chromium
154.0.8037.98). Its Linux distribution no longer has `libEGL.so` and `libGLESv2.so`: ANGLE is
linked into `libcef.so`, and the CMake file list (`cmake/cef_variables.cmake`,
`CEF_BINARY_FILES`) drops both. `tauri-bundler` 3.0.0-alpha.4 (`tauri-cli` 3.0.0-alpha.5) still
copies them unconditionally (`src/bundle/linux/debian.rs`, the `cef_files` list, and the same
list in `rpm.rs` and `appimage/sharun_cef.rs`), so `cargo tauri build --bundles deb` fails after
the build with:

    Bundling GitVolt_…_amd64.deb (…)
       Error failed to bundle project: `No such file or directory (os error 2)`

### Expected

Copy the two libraries only when the distribution has them (Chromium loads them from the module
directory only under `--use-dynamic-angle`: `ui/ozone/common/egl_util.cc`), or follow
`CEF_BINARY_FILES` / `CEF_RESOURCE_FILES`.

### Workaround we use

Point `CEF_PATH` at a copy of the distribution (links to its files) plus two empty shared
objects under the missing names, so the bundler finds them. Chromium never loads them.
