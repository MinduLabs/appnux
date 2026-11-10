# Cleaner: what it removes and why

Cleaning means caches and temporary files: data that the system or a tool rebuilds or downloads again by itself. Nothing the cleaner removes may change how a project, a service or an app behaves. The only cost allowed is time: a slower next install, build or first start.

Sections are fixed in code (`crates/appnux-core/src/cleaner.rs`). What shows up inside each one depends on what the machine has. An empty section is hidden.

## In scope

| Section | What | How it is cleaned | Ticked by default | Why it is safe |
|---|---|---|---|---|
| System caches | APT / pacman / DNF download cache | `apt-get clean`, `paccache -rk1` (`pacman -Sc` without pacman-contrib), `dnf clean packages` | Yes | The packages are installed already. `paccache -rk1` keeps one version of each package, so a downgrade still works. |
| System caches | systemd journal above the size set in Settings | `journalctl --vacuum-size=<N>M` | Yes | Keeps the newest logs. |
| Thumbnails | `~/.cache/thumbnails` | Delete folder | Yes | File managers make them again. |
| Developer tool caches | npm, Yarn, pip, uv, Go build cache | The tool's own command: `npm cache clean --force`, `yarn cache clean`, `pip cache purge`, `uv cache clean`, `go clean -cache` | Yes | Each tool downloads or rebuilds again when needed. npm documents its cache as self-healing, and says clearing it is only needed to reclaim space. |
| Developer tool caches | pnpm store | `pnpm store prune` | Yes | It removes only packages that no project on the machine uses, and pnpm documents it as having no side effects on projects. Only pnpm knows how much it frees, so no size is shown. |
| Developer tool caches | `~/.cargo/registry/src` | Delete folder | Yes | Cargo unpacks it again from `~/.cargo/registry/cache`, which is on the same disk. Nothing is downloaded. |
| Developer tool caches | `~/.cargo/registry/cache` | Delete folder | No | Downloaded again when a project needs a crate, so you need internet. |
| App caches | `~/.cache/<app>` of an installed app that is **not running** | Delete folder | No | The app rebuilds its cache. A cache in use is never offered, because deleting it under a running app can break that app. |
| Old snap revisions | Disabled revisions | `snap remove <name> --revision=<rev>` | No | Snap keeps the previous revision for rollback. Removing it drops that option, so it is your call. |
| Data of removed apps | `~/.var/app/<id>` and `~/snap/<name>` of apps that are no longer installed | Delete folder | No | This is data, not cache. It is listed so you can decide, and never ticked for you. |
| Trash | `~/.local/share/Trash` | Empty Trash | No | These are files you deleted. Emptying the Trash cannot be undone. |

## Out of scope, on purpose

| What | Why it is not cleaned |
|---|---|
| `node_modules`, Cargo `target/` and other project folders | A project needs them to run and build. Removing them breaks or slows your work. That is not cleaning. |
| Docker images, containers, volumes and build cache | Volumes hold data. Images and containers are services you run. |
| Test browsers in `~/.cache` (Cypress, Playwright, Puppeteer) | Test runs fail until you reinstall them. |
| Folders in `~/.cache` no installed app claims | Nothing tells what would break without them. |
| Config and data folders no app claims | They are often settings of command-line tools. |

## How removing works

Code: `crates/appnux-core/src/actions.rs`. Every item carries a typed operation (`Op`); nothing is built into a shell command.

- **Tool commands** (`npm cache clean --force`, `pnpm store prune`, `flatpak uninstall`…) run as you, and only for the programs in the `TOOLS` list.
- **Deleting and trashing** go through `guard()` first. A path must be inside home and inside one of the folders Appnux cleans (`~/.cache`, `~/.config`, `~/.local/share`, `~/.local/state`, `~/.var/app`, `~/snap`, `~/.npm`, `~/.cargo/registry`…), or be an app's own dot folder right in home. It is never one of those folders itself, never on the protected list (`.ssh`, `.gnupg`, keyrings, Trash, launchers, fonts, dconf, Flatpak installs, `.cargo/bin`, `.rustup`…), never a shared desktop folder, and never reached through a symlink. `remove_dir_all` does not follow symlinks.
- **No Trash.** Uninstall, Deep purge and Clean delete for good, as uninstalling does on other systems; the confirm dialog says so. Emptying the Trash skips `expunged`, GIO's own scratch folder.
- **AppImages and launchers:** each is one regular file. The AppImage must end in `.AppImage` and sit in home; the launcher must be in `~/.local/share/applications`.
- **Root operations** (package cache, journal, snaps, system packages) go through `pkexec appnux --root-helper` in one call, so the password is asked once. The helper checks every name and number again. Before removing a package it runs the package manager's dry run again, and refuses if the result differs from what you approved or now takes a core package. From an AppImage each root command gets its own `pkexec`, because root cannot read the image's private mount.
- A failed step is recorded and the others go on. Activity lists every step with ✓ or ✗ and the reason.

## Sources

- npm cache docs: https://docs.npmjs.com/cli/v7/commands/npm-cache/
- pnpm store prune: https://pnpm.io/cli/store
- pip cache: https://pip.pypa.io/page/cli/pip_cache/
- go clean: https://pkg.go.dev/cmd/go/internal/clean
- Cargo cache cleaning: https://blog.rust-lang.org/2023/12/11/cargo-cache-cleaning/
- Cargo registry src discussion: https://users.rust-lang.org/t/cargo-registry-src-keeps-growing/15375
- `~/.cache` contents are rebuildable, close apps first: https://bbs.archlinux.org/viewtopic.php?id=273652
