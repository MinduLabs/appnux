# Appnux — Kế hoạch sản phẩm & triển khai

> Nguồn: BRD "Linux Smart App Uninstaller & System Junk Cleaner" v1.0
> Ngày lập: 2026-11-09 · Rev 4 (đổi từ Tauri + React sang GTK4 + libadwaita)

---

## 0. Quyết định đã chốt

| Hạng mục | Quyết định |
|---|---|
| Tên | **Appnux** (App + Linux) |
| Nền tảng | **Chỉ Linux**, mọi distro thuộc 3 họ chính: **Debian/Ubuntu · Arch · Fedora/RHEL** |
| Hình thức | **Chỉ GUI**. Appnux thay thế việc gõ lệnh: mọi thao tác quản lý app và dọn dẹp đều làm trên giao diện. Không có CLI |
| Thương mại | **Miễn phí hoàn toàn**: không thu tiền, không bản Pro, không giới hạn tính năng |
| Giao diện | **GTK4 + libadwaita native** (gtk4-rs 0.11 với `v4_6`, libadwaita 0.9 với `v1_1`): chạy từ Ubuntu 22.04, Debian 12, Fedora 36+, RHEL 9 bản mới, Arch |
| RAM 80MB | **Đạt**: ~66 MB với renderer cairo (mặc định). Bật "Hardware acceleration" trong Settings thì dùng OpenGL, ~134 MB |
| Đã bỏ | Tauri v2 + React (đo ~280 MB vì WebKitGTK chạy nhiều process). Mã cũ nằm ở `archive/tauri-ui-2026-11-09.tar.gz` |

---

## 1. Tên

| Ứng viên | Kết quả | Kết luận |
|---|---|---|
| **Appnux** | Không tìm thấy dự án Linux trùng tên | ✅ |
| LAM | Trùng lệnh `lam` của LinuxAppsManager; trùng LDAP Account Manager | ❌ |
| AML | Trùng ACPI Machine Language (thuật ngữ kernel) | ❌ |

App ID `io.github.<owner>.Appnux`. Binary: `appnux` (app), `appnux-helper` (tác vụ cần quyền root, chỉ app gọi, người dùng không dùng trực tiếp).

---

## 2. Định vị

**Một màn hình duy nhất** để thấy mọi app trên máy (system package, Flatpak, Snap, AppImage), biết mỗi app thực sự chiếm bao nhiêu, gỡ sạch tận gốc, và dọn rác build của developer, **mà không phải mở terminal**.

| Đã có | Thiếu gì | Appnux bổ sung |
|---|---|---|
| BleachBit, Stacer | Không gắn với từng app, không gỡ tận gốc | Gắn mọi thứ theo app, có Deep Purge |
| Flatsweep | Chỉ Flatpak | Cả 4 nguồn × 3 họ distro |
| App Terminator, DeepPurge | Mới, chưa có dev junk, chưa có dashboard | Dev junk + dashboard dung lượng chính xác |
| kondo, npkill | Chỉ là tool dev, phần lớn dùng terminal | Gom chung vào cùng một UI |

Điểm khác biệt cốt lõi: **an toàn có kiểm chứng**. Luôn chạy thử (simulate) trước khi gỡ, mỗi đề xuất có mức tin cậy, có cảnh báo snapshot, và luôn có màn xem trước.

---

## 3. Phạm vi MVP

### 3.1. Trong phạm vi

| Mã | Chức năng | Ghi chú điều chỉnh so với BRD |
|---|---|---|
| BR-01 | Danh sách app hợp nhất, chi tiết, tìm kiếm, lọc | "DEB" → **System package** (apt / pacman / dnf) |
| BR-02 | Gỡ thường & Deep Purge | Luôn simulate trước khi gỡ |
| BR-03.1 | Quét dữ liệu mồ côi | Có mức tin cậy, protected paths |
| BR-03.2 | Dev junk: Cargo, Node, Docker | |
| BR-03.3 | Cache gói (theo họ distro) + journal | |
| BR-04 | Dashboard dung lượng + Quick Wins | Cảnh báo snapshot Btrfs |
| BR-05 | Safety blacklist, pkexec, xem trước | Blacklist 3 lớp, theo từng họ distro |

### 3.2. Ngoài phạm vi
- Task manager (kill, renice) — theo BRD.
- Quét ngoài các root an toàn đã định nghĩa — theo BRD.
- Tự dọn ngầm khi chưa có xác nhận — theo BRD.
- Cài app mới / app store: Appnux chỉ quản lý app đã cài.
- Distro ngoài 3 họ (openSUSE, NixOS, Gentoo...): app vẫn chạy được phần Flatpak/Snap/AppImage nếu cài được, nhưng không quản lý system package.

---

## 4. Điều chỉnh yêu cầu cho đo được và an toàn

### 4.1. NFR

| # | BRD | Điều chỉnh |
|---|---|---|
| NFR-01 | Quét 500GB < 5s | Tốc độ phụ thuộc **số inode**, không phụ thuộc GB. KPI: ≤ 1 triệu inode trong < 5s (warm cache), < 15s (cold cache), chỉ trên các root mục tiêu. Có cache kết quả + quét lại incremental |
| NFR-02 | < 80MB RAM | Đo **tổng PSS** của process (GTK4 chỉ có một process). Đạt ~66 MB với renderer cairo; theo dõi trong CI |
| NFR-02b | Binary < 20MB | Áp dụng cho `.deb`/`.rpm`/AUR. AppImage cho phép nặng hơn vì bundle thư viện |
| NFR-03 | GNOME, KDE, XFCE, i3, Hyprland × Ubuntu, Fedora, Arch | Giữ nguyên. Ma trận test chi tiết ở mục 10 |
| NFR-04 | Không xóa nhầm, graceful failure | Cụ thể hóa thành các quy tắc ở mục 6.7 |
| UAC-01 | "đúng 100%" | Định nghĩa: 100% app có `.desktop` hiển thị (không `NoDisplay=true`) trong XDG data dirs |

### 4.2. Rủi ro BRD chưa nhắc
1. Gỡ gói kéo theo dây chuyền (meta-package, autoremove) → bắt buộc simulate.
2. Flatpak dùng hardlink → nếu cộng size từng file thì con số bị phóng đại.
3. Snapshot Btrfs/Timeshift/Snapper → xóa file không giải phóng dung lượng.
4. Orphan scanner báo nhầm config của CLI tool, keyrings, ssh...
5. Symlink bị tráo giữa lúc xem trước và lúc xóa (TOCTOU).
6. Deep Purge trong `/var/lib` mà dựa vào đoán tên thì nguy hiểm.
7. Flatpak sandbox chặn hầu hết tính năng → không phát hành qua Flatpak.

---

## 5. Kiến trúc

### 5.1. Tổng thể

```
┌──────────────────────────── appnux (quyền user, một process) ─────────────────┐
│  app/ — GTK4 + libadwaita       │                                              │
│  Overview | Apps | Cleaner |  ◄─┼─► appnux-core (Rust, không phụ thuộc GTK)    │
│  Activity | Settings            │    apps, desktop, residue, procs, disk,      │
│  store.rs: State + Event bus    │    cleaner, plan, distro                     │
│  quét chạy trên worker thread   │                                              │
│  (gio::spawn_blocking)          │                                              │
└─────────────────────────────────┴──────────────────┬───────────────────────────┘
                                                     │ pkexec (Phase 1)
                               ┌─────────────────────▼────────────────────┐
                               │ appnux-helper (root, chạy rồi thoát)     │
                               │ Chỉ nhận tập op cố định, tự validate     │
                               │ lại, không bao giờ chạy shell            │
                               └──────────────────────────────────────────┘
```

### 5.2. Cấu trúc repo

```
appnux/
├─ crates/appnux-core/  # toàn bộ logic, test độc lập
├─ app/                 # giao diện GTK4: một module mỗi màn, store.rs, ui.rs (layout, dialog, skeleton)
├─ packaging/           # .desktop, icon, build-deb.sh, build-appimage.sh (sau này: polkit policy, PKGBUILD, rpm)
├─ archive/             # mã Tauri cũ (tham khảo)
└─ docs/                # PLAN, CLEANER, STYLE-GUIDE
```

Build: `packaging/build-deb.sh` → `dist/appnux_<ver>_amd64.deb`; `packaging/build-appimage.sh` → `dist/Appnux-<ver>-x86_64.AppImage`.

### 5.3. Provider

```rust
enum PkgKind { System(SysBackend), Flatpak, Snap, AppImage }
enum SysBackend { Apt, Pacman, Dnf }

trait PackageProvider: Send + Sync {
    fn kind(&self) -> PkgKind;
    fn is_available(&self) -> bool;
    fn list_apps(&self) -> Result<Vec<App>>;
    fn owned_paths(&self, app: &App) -> Result<Vec<PathBuf>>;
    fn protection(&self, app: &App) -> Result<Option<ProtectReason>>;
    fn plan_uninstall(&self, app: &App, mode: UninstallMode) -> Result<ActionPlan>; // gồm simulate
}
```

Chọn backend lúc runtime theo `/etc/os-release` (`ID`, `ID_LIKE`) và binary có sẵn. Mint/Pop → apt; Manjaro/EndeavourOS → pacman; Rocky/Alma → dnf.

### 5.4. Bảng lệnh System package *(viết theo trí nhớ, kiểm chứng ở Spike S2, nhất là dnf4 vs dnf5)*

| Thao tác | apt | pacman | dnf |
|---|---|---|---|
| File thuộc gói nào | `dpkg -S` | `pacman -Qo` | `rpm -qf` |
| File của gói | `dpkg -L` | `pacman -Ql` | `rpm -ql` |
| Phiên bản, size | `dpkg-query -W -f` | `pacman -Qi` | `rpm -q --qf` |
| Gỡ thường | `apt-get remove` | `pacman -R` | `dnf remove` |
| Gỡ tận gốc | `apt-get purge` | `pacman -Rns` + `.pacsave` | `dnf remove` + `.rpmsave` |
| Simulate | `apt-get -s remove` | `pacman -Rp --print` | `dnf remove --assumeno` |
| Gói bảo vệ | `Essential`, `Priority: required/important` | Dependency tree của `base`, `HoldPkg` | `/etc/dnf/protected.d/` |
| Cache | `apt-get clean` | `paccache -rk<N>` hoặc xóa trực tiếp | `dnf clean packages` |

| Nguồn | Danh sách | Size | Gỡ |
|---|---|---|---|
| Flatpak | `flatpak list --app` (user + system) | size của app + `~/.var/app/<id>` | `flatpak uninstall [--user] [--delete-data]` |
| Snap | snapd REST qua `/run/snapd.socket` | file `.snap` + `~/snap/<n>` + `/var/snap/<n>` | `snap remove [--purge]` |
| AppImage | `.desktop` có `Exec` trỏ tới `*.AppImage`; quét `~/Applications`, `~/AppImages`, `~/.local/bin` | size file + residue | xóa file + `.desktop` + icons |

### 5.5. Mô hình dữ liệu chính

```rust
struct App {
    id: AppId,                 // "<kind>:<native-id>"
    name: String, icon: Option<PathBuf>, version: String,
    kind: PkgKind, scope: Scope,          // User | System
    desktop_file: Option<PathBuf>,
    identifiers: Vec<String>,             // dùng cho residue matching
    size: SizeBreakdown,                  // binary / config / data / cache (lazy)
    protection: Option<ProtectReason>,
}
struct ActionPlan {
    id: PlanId, items: Vec<PlanItem>,
    pkg_ops: Vec<PkgOp>,                  // kết quả simulate
    free_now: u64, to_trash: u64,
    needs_root: bool, warnings: Vec<Warning>,  // snapshot, app đang chạy, ...
}
struct PlanItem { path: PathBuf, size: u64, confidence: Confidence, policy: DeletePolicy, selected: bool }
```

### 5.6. Luồng dữ liệu trong app

Không có IPC: giao diện gọi thẳng `appnux-core` trên worker thread (`gio::spawn_blocking`), kết quả quay về main loop rồi ghi vào `store::State`, sau đó phát `Event` (Apps, Footprints, Usage, Disk, Scanned, Selection, Activity, Prefs) cho các màn tự cập nhật. RAM theo app chỉ được đo lại (3 giây một lần) khi màn Apps đang mở.

### 5.7. Helper (root)

- Cài vào `/usr/libexec/appnux/appnux-helper`, polkit action `io.github.<owner>.appnux.manage` (`auth_admin_keep`).
- Giao thức: `pkexec appnux-helper` nhận **một op JSON qua stdin**, trả JSON qua stdout, rồi thoát.
- Tập op cố định: `pkg_remove`, `pkg_purge`, `pkg_cache_clean`, `journal_vacuum{max_size}`, `snap_remove`, `flatpak_uninstall_system`, `remove_paths{paths}` (chỉ trong allowlist: `/var/cache/{apt,pacman,dnf,libdnf5}`, `/var/lib/<pkg>` khi package manager xác nhận, `/var/snap/<n>`).
- Helper tự validate lại toàn bộ (không tin process user), gọi package manager bằng `Command` kèm args cố định, không qua shell.
- Người dùng bấm Cancel trong hộp thoại pkexec (exit code 126) → báo "Đã hủy", plan giữ nguyên (UAC-04).

---

## 6. Thiết kế các engine

### 6.1. Phát hiện app
Quét `.desktop` trong `$XDG_DATA_DIRS` + `~/.local/share/applications` + export của Flatpak/Snap → map ngược về provider: Flatpak/Snap dựa vào path export, AppImage dựa vào `Exec`, phần còn lại hỏi system package manager file đó thuộc gói nào. File `.desktop` không thuộc gói nào → nhóm "Không rõ nguồn" (vẫn hiển thị, chỉ cho xóa file .desktop).

### 6.2. Tính dung lượng
- Walk song song (`jwalk`/`ignore` + `rayon`), không theo symlink, không vượt mount point.
- Tính `st_blocks * 512`, dedupe theo `(dev, inode)` trong phạm vi mỗi phép tính.
- Snap: lấy size file `.snap`, không walk `/snap/*` (là squashfs mount).
- Cache kết quả vào `~/.cache/appnux/` theo mtime thư mục, invalidate khi thay đổi.

### 6.3. RAM theo app
`/proc/<pid>/cgroup` → scope (`app-gnome-*`, `app-kde-*`, `app-flatpak-<id>-*`, `snap.<name>.*`). Fallback theo `/proc/<pid>/exe` (cho i3/Hyprland). Cộng PSS từ `smaps_rollup`. Refresh mỗi 3s, chỉ khi màn Apps đang mở.

### 6.4. Residue & orphan
- Identifier: package name, app ID và phần cuối của ID, binary names, `StartupWMClass`, tên `.desktop`, alias trong `rules/aliases.toml`.
- Root: `~/.config`, `~/.local/share`, `~/.local/state`, `~/.cache`, `~/.var/app`, `~/snap`, dotdir cấp 1 trong home.
- Mức tin cậy:

| Mức | Điều kiện | Mặc định |
|---|---|---|
| Cao | Khớp chính xác ID (`~/.var/app/<id>`, `~/snap/<n>`), hoặc package manager xác nhận | Tick sẵn |
| Trung bình | Tên thư mục khớp chính xác identifier | Không tick |
| Thấp | Khớp một phần | Không tick, ẩn sau "Hiện thêm" |

- Loại trừ: protected paths; thư mục còn được dùng bởi app khác vẫn đang cài; thư mục có mtime < 7 ngày.
- Orphan = thư mục trong các root mà **không** khớp app nào đang cài **và không** nằm trong `rules/known_cli.toml` (cargo, rustup, npm, nvm, pip, git, ssh, gnupg, docker...) hay `rules/protected.toml`.

### 6.5. Dev junk
| Loại | Phát hiện | Dọn |
|---|---|---|
| Cargo target | `target/` có `CACHEDIR.TAG` hoặc nằm cạnh `Cargo.toml` | Xóa thư mục |
| Cargo cache | `~/.cargo/registry/{cache,src}`, `~/.cargo/git/checkouts` | Xóa thư mục |
| node_modules | Nằm cạnh `package.json`, mtime project > N ngày (mặc định 30) | Xóa thư mục |
| npm/pnpm/yarn store | `~/.npm/_cacache`, pnpm store, yarn cache | Gọi lệnh chính chủ nếu có, không thì xóa |
| Docker | Socket: `$DOCKER_HOST` → rootless → `/var/run/docker.sock` | API (`bollard`): prune dangling images, unused volumes, build cache |

Dev roots mặc định là `~` (giới hạn độ sâu, bỏ qua thư mục ẩn hệ thống), chỉnh được trong Settings. Không truy cập được Docker → hiển thị lý do, không tự leo quyền.

### 6.6. System junk
Cache gói theo họ distro (bảng 5.4) + `journalctl --vacuum-size=<N>` (mặc định giữ 100MB). Cả hai đều qua helper.

### 6.7. Chính sách xóa & an toàn

**Cách xóa theo loại dữ liệu:**
| Loại | Cách xóa |
|---|---|
| Tạo lại được (cache, target, node_modules, Docker, cache gói, journal) | Xóa vĩnh viễn |
| Dữ liệu người dùng (`~/.config`, `~/.local/share`, `~/.var/app`, `~/snap`) | **Mặc định Thùng rác** (chuẩn freedesktop), có toggle xóa vĩnh viễn |
| Vùng hệ thống | Vĩnh viễn qua helper, sau khi đã simulate hoặc package manager xác nhận |

Màn xem trước hiển thị riêng "Giải phóng ngay" và "Chuyển vào Thùng rác". Appnux ghi lại những item nó đã đưa vào Trash; Dashboard có Quick Win "Dọn vĩnh viễn các mục Appnux đã đưa vào Thùng rác" (vẫn phải xác nhận).

**Quy tắc an toàn (không được vi phạm):**
1. Safety blacklist 3 lớp: metadata của distro + `rules/blacklist.toml` (kernel, systemd, glibc, bash, sudo, polkit, package manager, display manager, DE đang chạy) + simulate có chạm vào gói bảo vệ thì chặn. Gói bị bảo vệ → nút Purge disabled kèm tooltip lý do.
2. Mọi path phải nằm trong allowlist root sau khi canonicalize; xóa bằng `unlinkat` + `O_NOFOLLOW`; kiểm tra lại ngay lúc xóa.
3. Không bao giờ xóa: `$HOME`, root của một XDG dir, mount point, path ngoài allowlist.
4. Lỗi từng item (bị khóa, thiếu quyền) → ghi log, bỏ qua, chạy tiếp; tổng kết cuối cùng liệt kê item lỗi.
5. Có snapshot Btrfs/Timeshift/Snapper → cảnh báo trong xem trước rằng dung lượng có thể không giảm.
6. App đang chạy → cảnh báo, đề nghị đóng app trước khi gỡ.
7. Mọi thao tác ghi vào `~/.local/state/appnux/activity.jsonl` (hiển thị ở màn Activity log).

---

## 7. Giao diện

| Màn | Nội dung |
|---|---|
| **Dashboard** | Dung lượng đĩa, phân bổ 5 nhóm (System apps / Config / Cache / Dev junk / Free), treemap drill-down, Quick Wins |
| **Apps** | Bảng: icon, tên, loại gói, phiên bản, tổng size, RAM; search real-time; filter All / System / Flatpak / Snap / AppImage; panel chi tiết; nút Gỡ / Deep Purge |
| **Cleaner** | 3 tab: Dữ liệu mồ côi · Developer · Hệ thống; mỗi tab: Quét → danh sách nhóm theo mức tin cậy → xem trước → dọn |
| **Preview** (modal dùng chung) | Gói sẽ bị gỡ (từ simulate), danh sách path + size + mức tin cậy + Trash/vĩnh viễn, cảnh báo, tổng giải phóng, nút xác nhận |
| **Activity log** | Lịch sử thao tác, kết quả, item lỗi |
| **Settings** | Dev roots, ngưỡng tuổi node_modules, dung lượng journal giữ lại, mặc định Trash/vĩnh viễn, ngôn ngữ (EN/VI), theme theo hệ thống |

Luồng chuẩn của mọi thao tác xóa: **Chọn → Plan (simulate/quét) → Preview → Xác nhận → (pkexec nếu cần) → Kết quả → Activity log**.

---

## 8. Phân phối

| Kênh | Họ distro | Ghi chú |
|---|---|---|
| `.deb` | Debian/Ubuntu | Phụ thuộc `libwebkit2gtk-4.1`; cài kèm helper + polkit policy |
| `.rpm` | Fedora/RHEL | Như trên. **Cần kiểm tra RHEL 9 có `webkit2gtk4.1` không** |
| AUR `appnux-bin` | Arch | PKGBUILD lấy binary từ GitHub Releases |
| AppImage | Mọi distro (dự phòng) | Nặng hơn; không cài được polkit policy → dùng prompt pkexec chung |
| Flatpak | — | Không phát hành (sandbox chặn tính năng) |

Build trong CI trên base glibc cũ nhất cần hỗ trợ (Ubuntu 22.04). Phát hành qua GitHub Releases. Sau đó có thể mở thêm PPA / COPR.

---

## 9. Lộ trình (1 dev full-time, khoảng 11 tuần)

### Phase 0 — Spikes (1 tuần)
| Spike | Mục tiêu | Kết quả cần có |
|---|---|---|
| S1 | ~~Tauri v2 + React~~ → đã làm: Tauri ~280 MB, GTK4 ~66 MB (cairo) | **Xong: chọn GTK4** |
| S2 | Prototype list app trên Ubuntu 24.04 / Fedora / Arch (container hoặc VM) + máy này | Bảng 5.4 đã kiểm chứng |
| S3 | Benchmark scanner + dedupe hardlink | Chốt KPI NFR-01 |
| S4 | pkexec + helper + polkit trên cả 3 họ, luồng Cancel | Mẫu helper chạy được |

### Phase 1 — Nền móng (1.5 tuần)
Workspace + GTK4 app + i18n (gettext), CI (fmt, clippy, test, build deb/rpm/AppImage, đo RAM), `safety/` (viết test trước), `actions/` (plan, executor, Trash/vĩnh viễn, activity log, graceful failure), `distro/` (os-release, phát hiện snapshot), helper + polkit.
**Xong khi:** test của `safety/` pass toàn bộ, build ra được 3 loại gói.

### Phase 2 — Quản lý ứng dụng (3.5 tuần) · BR-01, BR-02 · UAC-01, UAC-02
- System providers apt / pacman / dnf (simulate + gói bảo vệ).
- Flatpak, Snap, AppImage; resolve icon; size calculator chạy nền; procmap.
- UI Apps + Preview + gỡ thường / Deep Purge.
**Xong khi:** đạt UAC-01, UAC-02 trên cả 3 họ distro; `systemd` có nút Purge bị disabled.

### Phase 3 — Cleaner (2 tuần) · BR-03 · UAC-03, UAC-04
Orphan scanner, dev junk, system junk qua helper, UI Cleaner.
**Xong khi:** đạt UAC-03 (dung lượng giải phóng khớp dự báo, sai số < 2%), UAC-04 (Cancel pkexec an toàn).

### Phase 4 — Dashboard (1 tuần) · BR-04
Overview, treemap, Quick Wins (snap revision cũ, Flatpak unused runtimes, cache gói, mục Appnux đã đưa vào Trash), cảnh báo snapshot.

### Phase 5 — Hoàn thiện & phát hành (2 tuần)
Chạy ma trận test (mục 10), sửa lỗi, đo lại RAM/size, polish UI (empty/error/loading states), viết README + hướng dẫn cài cho từng họ distro, release v1.0.

**Sau v1.0** (chưa cam kết): zypper/openSUSE, mở rộng `rules/` nhờ đóng góp của cộng đồng, thêm loại dev junk (Gradle, Python venv, Go...).

---

## 10. Kiểm thử

| Loại | Nội dung |
|---|---|
| Unit | `safety/`: symlink escape, `..`, mount point, path bị đổi giữa preview và execute, protected path. Residue matcher trên fixture |
| Fixture | Home giả trong tmpdir: app đã gỡ, CLI tools, hardlink, node_modules cũ/mới |
| Integration | Container cho apt / pacman / dnf: cài app thật → Deep Purge → assert không còn dấu vết. Snap chạy trong VM |
| Size accuracy | So `statvfs` trước/sau, sai số < 2% (bỏ qua khi có snapshot) |
| Dry-run | `APPNUX_DRY_RUN=1`: chỉ ghi log, không xóa (dùng khi dev/QA) |
| Ma trận | Ubuntu 22.04/24.04, Debian 12, Mint · Fedora mới nhất, Rocky/Alma 9 · Arch, EndeavourOS, Manjaro × GNOME, KDE, XFCE, i3, Hyprland |
| Hiệu năng | RAM (tổng PSS) và thời gian quét đo trong CI, fail nếu tăng > 10% so với baseline |

---

## 11. Rủi ro

| Rủi ro | Ảnh hưởng | Giảm thiểu |
|---|---|---|
| Xóa nhầm dữ liệu người dùng | Rất cao | Mức tin cậy, mặc định Trash, protected paths, xem trước, test `safety/` |
| Gỡ gói làm hỏng hệ thống | Rất cao | Simulate + blacklist 3 lớp |
| RAM vượt 80MB | Thấp | GTK4 + cairo đo được ~66 MB; theo dõi trong CI |
| Lệnh dnf4/dnf5, pacman-contrib khác nhau | Trung bình | Phát hiện lúc runtime, test ma trận |
| RHEL 9 thiếu webkit2gtk-4.1 | Trung bình | Kiểm tra ở S2; nếu thiếu thì RHEL dùng AppImage |
| Residue matching bỏ sót hoặc báo nhầm | Trung bình | `rules/*.toml` mở để cộng đồng bổ sung |

---

## 12. Truy vết yêu cầu

| BRD | Module | Phase | Nghiệm thu |
|---|---|---|---|
| BR-01 | providers, scanner, procmap, UI Apps | 2 | UAC-01 |
| BR-02 | providers.plan_uninstall, residue, actions | 2 | UAC-02 |
| BR-03.1 | residue (orphan) | 3 | — |
| BR-03.2 | devjunk | 3 | UAC-03 |
| BR-03.3 | sysjunk, helper | 3 | UAC-04 |
| BR-04 | scanner, UI Dashboard | 4 | — |
| BR-05 | safety, helper, Preview | 1–3 | UAC-04 |
| NFR-01/02 | scanner, FE tối ưu, CI benchmark | 0, 5 | Benchmark CI |
| NFR-03 | distro, providers | 2, 5 | Ma trận test |
| NFR-04 | safety, actions | 1 | Unit + integration |

---

## 13. Còn cần chốt

| # | Câu hỏi | Đề xuất |
|---|---|---|
| Q5 | License. Bạn muốn source mở nhưng không hoàn toàn tự do; app miễn phí nên "giới hạn" ở đây là gì: chặn người khác đổi tên rồi phát hành lại, hay chặn dùng thương mại? | **GPL-3.0 + giữ bản quyền tên/logo "Appnux"**: fork phải mở source theo GPL, phải đổi tên và logo, không được đóng source. Vẫn vào được AUR/PPA/COPR. Nếu muốn chặt hơn (cấm dùng thương mại) thì dùng PolyForm Noncommercial, nhưng khi đó không còn là open source và không vào được repo chính thức của distro |
| Q6 | Owner cho App ID / repo GitHub? | — |
