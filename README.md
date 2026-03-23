# lgtv-service

A Windows service that turns off an LG C4 TV when the PC goes to sleep and turns it back on when the PC wakes up.

## How it works

- **Sleep** → connects to the TV over its local WebSocket API (`wss://TV_IP:3001`) and sends a power-off command
- **Wake** → sends a Wake-on-LAN magic packet to the TV's MAC address

## Requirements

- Windows 10/11
- LG webOS TV (C-series or similar) on the same local network
- The TV must have **Quick Start+** enabled (keeps the network interface active in standby so WoL works)

## Setup

### 1. Enable Quick Start+ on the TV

`Settings → All Settings → General → Quick Start+` → **On**

This keeps the TV's network interface alive in standby so it can receive Wake-on-LAN packets.

### 2. Create the config file

Create the directory `C:\ProgramData\lgtv-service\` and put a `config.toml` file in it.
Copy [`config.example.toml`](config.example.toml) as a starting point:

```toml
[tv]
ip  = "192.168.1.50"   # TV's local IP address (assign a static lease in your router)
mac = "A8:23:FE:01:02:03"  # TV's MAC address

[timeouts]
connect_secs = 3
ack_secs     = 2
```

**Finding your TV's IP and MAC:**
`TV Settings → All Settings → General → About This TV → Network Information`

### 3. Build the binary

```powershell
cargo build --release
```

Or grab the pre-built binary and place it somewhere permanent, e.g.:
```
C:\Program Files\lgtv-service\lgtv-service.exe
```

### 4. Pair with the TV

The TV must be **on** for this step.

```powershell
.\lgtv-service.exe pair
```

A prompt will appear on your TV screen. Select **Allow** with your remote. The client key is saved automatically to `C:\ProgramData\lgtv-service\client_key.txt`.

### 5. Install the Windows service

First copy the binary to a permanent location — the service will run from wherever it is when you run `install`:

```powershell
New-Item -ItemType Directory "C:\Program Files\lgtv-service"
Copy-Item .\target\release\lgtv-service.exe "C:\Program Files\lgtv-service\lgtv-service.exe"
```

Then from an **elevated** (Administrator) prompt:

```powershell
& "C:\Program Files\lgtv-service\lgtv-service.exe" install
sc.exe start lgtv-service
```

The service is set to start automatically on boot.

## Commands

| Command | Description |
|---|---|
| `install` | Register the Windows service (requires elevation) |
| `uninstall` | Remove the Windows service (requires elevation) |
| `run` | Start the service loop (called internally by the SCM) |
| `pair` | Interactively pair with the TV and save the client key |
| `test-power` | Turn the TV off, wait 30 s, then turn it back on — for testing |
| `test [IP]` | Raw TCP probe to diagnose WebSocket connectivity |

## Troubleshooting

**TV doesn't turn off on sleep**
- Check that the service is running: `sc.exe query lgtv-service`
- Check Windows Event Viewer → Windows Logs → Application → source `lgtv-service` for errors
- Run `.\lgtv-service.exe test-power` to verify the connection works manually

**TV doesn't turn on after wake**
- Make sure **Quick Start+** is enabled on the TV (see step 1)
- Verify the MAC address in `config.toml` is correct
- Some routers block broadcast packets — try assigning a directed broadcast address instead of `255.255.255.255` (not currently configurable, open an issue)

**Pairing prompt never appears**
- Make sure the TV and PC are on the same network
- Run `.\lgtv-service.exe test` to verify port 3001 is reachable
- Try toggling Quick Start+ off and on, then power-cycling the TV

**Re-pairing**
Delete `C:\ProgramData\lgtv-service\client_key.txt` and run `lgtv-service.exe pair` again.

## Uninstalling

```powershell
sc.exe stop lgtv-service
.\lgtv-service.exe uninstall
```

Then delete `C:\Program Files\lgtv-service\` and `C:\ProgramData\lgtv-service\`.
