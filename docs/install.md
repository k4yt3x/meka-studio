# Installing Meka Studio

Meka Studio comes two ways. The [desktop app](#the-desktop-app), Meka Studio Desktop, is for the computer you work on: it runs a Meka of its own, and needs nothing else set up. The server runs on a Meka host, for any browser or phone to reach; the numbered steps set it up.

## The desktop app

Download the installer for your computer from the [releases](https://github.com/k4yt3x/meka-studio/releases): `meka-studio-desktop-x86_64-linux.deb` or `meka-studio-desktop-x86_64-linux.AppImage`, `meka-studio-desktop-aarch64-macos.dmg` for Apple silicon, or `meka-studio-desktop-x86_64-windows-setup.exe`, or on Windows the [portable copy](#the-portable-copy-on-windows) `meka-studio-desktop-x86_64-windows-portable.zip`. These are the systems Meka has releases for. It is not signed: on macOS, move it to Applications and run `xattr -dr com.apple.quarantine "/Applications/Meka Studio Desktop.app"` before opening it; on Windows, SmartScreen asks before the installer or the app runs. On Linux the `.deb` installs the WebKitGTK 4.1 it needs, and the AppImage carries its own.

The Windows installer installs the app for you alone, in `%LOCALAPPDATA%\Programs\Meka Studio Desktop`, unless you choose all users, which asks for an administrator. It installs Microsoft's WebView2 runtime, which draws the window, if Windows lacks it, as Windows 10 may. Each download includes the app's `LICENSE` and `THIRD-PARTY-NOTICES.txt`.

The app runs the Meka it comes with, which it starts and stops itself, and keeps that Meka's config and sessions in folders of its own. It reads nothing of another Meka on the computer, so it starts with no providers: add one in **Settings → Providers**.

| Folder                                       | Linux                                        | macOS                                           | Windows                                      |
| -------------------------------------------- | -------------------------------------------- | ----------------------------------------------- | -------------------------------------------- |
| Settings, and Meka's config in `meka-config` | `~/.config/meka-studio-desktop`              | `~/Library/Application Support/bot.meka.studio` | `%APPDATA%\Meka Studio Desktop`              |
| Data, with Meka's sessions in `meka-data`    | `~/.local/share/meka-studio-desktop`         | `~/Library/Application Support/bot.meka.studio` | `%APPDATA%\Meka Studio Desktop`              |
| Logs                                         | `~/.local/share/meka-studio-desktop/logs`    | `~/Library/Logs/bot.meka.studio`                | `%LOCALAPPDATA%\Meka Studio Desktop\logs`    |
| The window's own storage                     | `~/.local/share/meka-studio-desktop/webview` | Where macOS keeps it                            | `%LOCALAPPDATA%\Meka Studio Desktop\webview` |

The `meka` command reaches the app's sessions with `MEKA_CONFIG_DIR` and `MEKA_DATA_DIR` set to its `meka-config` and `meka-data` folders, if it is the same version as the app's Meka, which **Settings → About** shows: a Meka refuses sessions a newer one has stored.

On Linux, `XDG_CONFIG_HOME` moves the first of these folders and `XDG_DATA_HOME` the others.

To upgrade, install the new version over the old one; the Windows installer quits the installed app first if it runs. Uninstalling leaves the folders above, with your settings and sessions. The app looks for a newer release once a day, and says when there is one; **Settings → General** turns that off.

One copy of the app runs at a time. Starting it again shows the running one's window; starting another copy shows it too, with a notification of where the running copy is.

### The portable copy on Windows

The portable zip holds the same app, with an empty `.portable` file beside `meka-studio-desktop.exe`. That file makes it keep everything in a `data` folder beside it: its settings, Meka's config in `meka-config` and sessions in `meka-data`, the logs in `logs`, and the window's own storage in `webview`. Unzip it to a folder you can write to, such as one on a USB drive; in a folder it cannot write to, such as `C:\Program Files`, it says so and does not start. **Settings → About** shows it as portable.

It writes nothing to your profile's folders. Windows shows a program's notifications only under a name registered for it, which the installer's Start menu shortcut registers, so while system notifications are on, a portable copy registers that name under `HKEY_CURRENT_USER\Software\Classes\AppUserModelId\bot.meka.studio` and removes it when it quits or when they are turned off. Turn system notifications off in **Settings → Notifications** to keep it out of the registry.

To upgrade it, quit it and unzip the new version over it; the zip has no `data` folder, so yours stays as it is.

If the window stays blank or flickers on Linux with an NVIDIA graphics card, start the app with `WEBKIT_DISABLE_DMABUF_RENDERER=1` set: WebKitGTK, which draws the window, does not always work with NVIDIA's driver.

## The server

Meka Studio runs on the Meka host, beside Meka. Meka listens on loopback only; Meka Studio signs you in, serves the interface, and passes your requests to Meka with its token. Only Meka Studio is reachable from your network or the internet, through a TLS proxy or a tunnel in front of it.

```
browser ──HTTPS──▶ Cloudflare Tunnel or Caddy ──▶ Meka Studio (loopback) ──▶ meka serve (loopback)
```

Meka Studio runs on Linux, macOS and Windows. Each release states which Meka versions it supports; this one needs Meka 0.95.0 or newer. Files marks git's changes where git 2.31 or newer is installed. The steps show Linux and macOS commands, with Windows' where they differ.

## 1. Get Meka Studio

Download the binary for your machine from the [releases](https://github.com/k4yt3x/meka-studio/releases): `meka-studio-x86_64-linux`, `meka-studio-aarch64-linux`, `meka-studio-aarch64-macos` for Apple silicon, or `meka-studio-x86_64-windows.exe`. Check its checksum against the SHA-256 the release page shows beside it, and put it where the service you choose in step 5 runs it from: `/usr/local/bin` for a system unit or on macOS, `~/.local/bin` for a user unit.

```sh
sudo install -m 755 meka-studio-x86_64-linux /usr/local/bin/meka-studio
```

On macOS, download it with `curl -LO`: a binary downloaded in a browser is quarantined, and runs only after `xattr -d com.apple.quarantine` on it. On Windows, put it in a folder of its own as `meka-studio.exe`:

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Programs\meka-studio" | Out-Null
Move-Item meka-studio-x86_64-windows.exe "$env:LOCALAPPDATA\Programs\meka-studio\meka-studio.exe"
Unblock-File "$env:LOCALAPPDATA\Programs\meka-studio\meka-studio.exe"
$path = [Environment]::GetEnvironmentVariable('Path', 'User')
[Environment]::SetEnvironmentVariable('Path', "$path;$env:LOCALAPPDATA\Programs\meka-studio", 'User')
```

A new PowerShell then runs it as `meka-studio`.

Or build it: install Node.js 24 and the Rust toolchain, then

```sh
(cd web && npm ci && npm run build)
cargo build --release --locked -p meka-studio
sudo install -m 755 target/release/meka-studio /usr/local/bin/meka-studio
```

The interface is built into the binary, so build `web/` first. With [just](https://github.com/casey/just), `just build` does both. On Windows, building the bundled OpenSSL needs Perl, such as Strawberry Perl. A binary built on another machine needs a glibc at least as new as that machine's; `cargo build --release --locked -p meka-studio --target x86_64-unknown-linux-musl`, with `musl-tools` installed and the target added (`rustup target add x86_64-unknown-linux-musl`), builds one that runs on any Linux, as the releases are.

## 2. Give Meka Studio a Meka token

Make a token Meka Studio alone holds, readable only by the user Meka and Meka Studio run as:

```sh
mkdir -p ~/.config/meka-studio
head -c 32 /dev/urandom | base64 | tr -d '/+=' > ~/.config/meka-studio/meka-token
chmod 600 ~/.config/meka-studio/meka-token
```

On Windows, in PowerShell:

```powershell
New-Item -ItemType Directory -Force "$env:APPDATA\meka-studio" | Out-Null
$bytes = [byte[]]::new(32)
[Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
[Convert]::ToBase64String($bytes) -replace '[/+=]', '' | Set-Content -NoNewline "$env:APPDATA\meka-studio\meka-token"
```

Add it to Meka's `config.toml` with every scope, and keep Meka on loopback. Meka reads the same file, at its absolute path, and warns unless only its owner can read it. Meka Studio is Meka's only client, so Meka needs no CORS origins:

```toml
[serve]
bind = "127.0.0.1:8080"

[[serve.tokens]]
token_file = "/home/<you>/.config/meka-studio/meka-token"
description = "Meka Studio"
scopes = [
  "sessions:r", "sessions:w",
  "memory:r", "memory:w",
  "skills:r", "skills:w",
  "schedule:r", "schedule:w",
  "mcp:r", "mcp:w",
  "config:r", "config:w",
]
```

`config:w` lets Meka Studio add accounts, profiles and MCP servers; an MCP server's command runs as Meka's user. A Meka with no profile yet can be set up from **Providers** once Meka Studio runs.

## 3. Configure Meka Studio

Copy [`config/meka-studio.toml`](../config/meka-studio.toml) to `~/.config/meka-studio/config.toml`, or `%APPDATA%\meka-studio\config.toml` on Windows, and set the following, uncommenting the optional ones you use. Write a Windows path in single quotes, `'C:\Users\you\AppData\Roaming\meka-studio\meka-token'`: in double quotes a backslash starts an escape.

- `public_origin`: the address you open Meka Studio at, such as `https://meka.example.com`. Passkeys work only on this hostname, so use the same name at home and away (see [one hostname](#one-hostname)).
- `meka.url`: Meka's loopback address.
- `meka.token_file`: the token file from step 2, which Meka Studio reads again every minute and whenever Meka refuses the token, so changing it needs no restart. Or write the token itself as `meka.token`; Meka Studio then warns unless only its owner can read the config.
- `push.subject`: a `mailto:` or `https:` address push services can reach you at. Apple refuses anything else. Without it, push notifications are off.

Check it:

```sh
meka-studio check
```

## 4. Add yourself

Meka Studio has one user. Add them, choosing a password of at least 10 characters:

```sh
meka-studio user add <username>
```

After signing in, add a passkey in **Settings → Account → Passkeys**. `meka-studio user passwd` sets a forgotten password and signs every device out, unless given `--keep-devices`; `meka-studio user reset-passkeys` removes lost passkeys and signs every device out. A running Meka Studio ends those devices' open connections within a minute.

## 5. Run Meka Studio as a service

Meka runs as its own service, set up as [Meka's documentation](https://github.com/k4yt3x/meka) describes, so restarting or upgrading Meka Studio never interrupts a turn. Run Meka Studio as the user Meka runs as: Files and the terminal act as that user, and it reads the same token file. On Linux either unit works:

- **A system unit**, [`deploy/system/meka-studio.service`](../deploy/system/meka-studio.service), starts at boot like the system's other services. Set `User=` and `Group=`, and the binary's path if it is not `/usr/local/bin/meka-studio`, then:

  ```sh
  sudo cp deploy/system/meka-studio.service /etc/systemd/system/
  sudo systemctl daemon-reload
  sudo systemctl enable --now meka-studio
  ```

  Its processes have no user session, so `systemctl --user` fails in its terminal. Uncomment `XDG_RUNTIME_DIR` with the user's id, and run `loginctl enable-linger <user>` so the folder it names is always there.

- **A user unit**, [`deploy/user/meka-studio.service`](../deploy/user/meka-studio.service), runs from `~/.local/bin/meka-studio` in your own service manager:

  ```sh
  cp deploy/user/meka-studio.service ~/.config/systemd/user/
  loginctl enable-linger
  systemctl --user daemon-reload
  systemctl --user enable --now meka-studio
  ```

  `enable-linger` starts your services at boot and keeps them running without a login session.

On **macOS**, the launch agent [`deploy/macos/com.github.k4yt3x.meka-studio.plist`](../deploy/macos/com.github.k4yt3x.meka-studio.plist) runs Meka Studio while you are logged in. Set your username in its log path, and the binary's path if it is not `/usr/local/bin/meka-studio`, then:

```sh
cp deploy/macos/com.github.k4yt3x.meka-studio.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.github.k4yt3x.meka-studio.plist
```

On **Windows**, a scheduled task starts Meka Studio at boot, as you, without a window. Register it in an administrator PowerShell:

```powershell
$action = New-ScheduledTaskAction -Execute "$env:LOCALAPPDATA\Programs\meka-studio\meka-studio.exe" -Argument serve
$trigger = New-ScheduledTaskTrigger -AtStartup
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask meka-studio -Action $action -Trigger $trigger -Principal $principal -Settings $settings
Start-ScheduledTask meka-studio
```

Its terminals run PowerShell unless **Settings → General** or `terminal.shell` names another shell.

## 6. Reach it over HTTPS

Browsers offer passkeys, secure cookies, and push notifications only over HTTPS (or on `localhost`).

**Cloudflare Tunnel** reaches a home server without opening a port, with a certificate every device trusts. With `cloudflared` installed and logged in:

```yaml
# ~/.cloudflared/config.yml
tunnel: <tunnel id>
credentials-file: /home/<you>/.cloudflared/<tunnel id>.json
ingress:
  - hostname: meka.example.com
    service: http://127.0.0.1:8090
  - service: http_status:404
```

Event streams and the terminal's WebSocket pass through the tunnel. Meka sends a keep-alive every 20 seconds and Meka Studio pings each terminal every 30, well inside Cloudflare's 100-second idle limit. Cloudflare allows request bodies of 100 MB on its free plan; Files uploads in 8 MiB pieces, so larger files upload too. Cloudflare Access can add another sign-in in front, but Meka Studio does not need it.

**Caddy** on your LAN serves the same name with a certificate from Cloudflare's DNS challenge:

```
meka.example.com {
	tls {
		dns cloudflare {env.CF_API_TOKEN}
	}
	reverse_proxy 127.0.0.1:8090
}
```

Add the proxy's address to `trusted_proxies` so the sign-in log and rate limits see each client's address rather than the proxy's. Meka Studio reads it from `X-Forwarded-For`, taking the last address there that is not a trusted proxy's, which is the one the nearest proxy saw. It does not read `CF-Connecting-IP`: a proxy that does not set that header passes on whatever a client sent.

### One hostname

A passkey belongs to the hostname it was made on. Reach Meka Studio by the same name everywhere: through the tunnel away from home, and through Caddy at home, with your LAN's DNS answering that name with the server's address.

## 7. Install it on your phone

Open the address in Safari, choose **Share → Add to Home Screen**, and open Meka Studio from the Home Screen. Sign in, then turn on **Settings → Notifications → Push notifications** there: iOS offers push notifications only to apps on the Home Screen, and asks for permission when you tap the switch. The app's storage is separate from Safari's, so it signs in on its own.

## Upgrading

Install the new binary and restart Meka Studio: `systemctl --user restart meka-studio`, or `sudo systemctl restart meka-studio` for a system unit; `launchctl kickstart -k gui/$(id -u)/com.github.k4yt3x.meka-studio` on macOS; `Stop-ScheduledTask meka-studio` before replacing the binary and `Start-ScheduledTask meka-studio` after on Windows. Its release notes say which Meka versions it supports; upgrade Meka when they ask, and **Settings** says whether the two match.
