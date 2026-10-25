# External probe (LR-17)

A market's own host cannot report that it is gone. The external probe runs every
minute on another machine and tells the owner on Telegram when a market's public
servers stop answering. Host-side alerts (unit failures, disk, backups, stale match
workers) are in [ops/vps/ru/README.md › Monitoring and alerts](../vps/ru/README.md#monitoring-and-alerts).

Each round, [market_probe.py](../../scripts/market_probe.py) checks:

| Check | Passes when |
| --- | --- |
| `lobby_udp` | the lobby answers the UDP compatibility check (the JSON exchange a game sends before anything else, [protocols](../../docs/architecture/protocols.md)) with a valid report for the probe's nonce within 4 s (the probe is resent every 0.5 s) |
| `account_api` | `GET <account_api>/health/ready` answers 200 |
| `portal` | `GET <portal>/` answers 200–399 |

A check that fails 2 rounds in a row (`OMOBA_PROBE_FAILURES`) sends one Telegram
message, a reminder every 60 minutes while it stays down
(`OMOBA_PROBE_REPEAT_MINUTES`) and one message when it answers again. Every alert
also goes to the journal (`journalctl -u omoba-probe@ru`). The probe sends nothing
but these requests; its messages name only the check, the public URL and the error.

The names come from [scripts/markets.py](../../scripts/markets.py) and nowhere else:
`market_probe.py config --market <market>` writes them into the probe host's settings
file. After a rename in `markets.py`, write that file again and copy it over.

## Where it runs (owner)

Any always-on Linux machine with systemd and Python 3.9+ **outside** the market's host
and, ideally, outside its provider: a small VPS, a home server, or another OMOBA host.
It never runs on the RU host itself. For the RU market a probe outside Russia also
shows whether players abroad can reach the servers; nothing personal is sent, so it
may run anywhere. Choosing the machine is an owner step (launch-readiness owner
checklist, LR-17).

## Install on the probe host

On the Mac, from a checkout of the release:

```sh
python3 scripts/market_probe.py check --market ru   # one round from here, no alerts
python3 scripts/market_probe.py config --market ru --out ru.env
# fill OMOBA_TELEGRAM_BOT_TOKEN and OMOBA_TELEGRAM_CHAT_ID in ru.env (the RU host's bot)
scp scripts/market_probe.py ops/probe/omoba-probe@.service ops/probe/omoba-probe@.timer ru.env <probe-host>:
rm ru.env
```

On the probe host, as root:

```sh
install -d -m 0755 /opt/omoba-probe && install -m 0644 market_probe.py /opt/omoba-probe/
install -d -m 0700 /etc/omoba-probe && install -m 0600 ru.env /etc/omoba-probe/ru.env
install -m 0644 omoba-probe@.service omoba-probe@.timer /etc/systemd/system/
systemctl daemon-reload
python3 /opt/omoba-probe/market_probe.py check --env-file /etc/omoba-probe/ru.env
systemctl enable --now omoba-probe@ru.timer
```

`check` exits 0 only when every check passes. The timer runs
`omoba-probe@ru.service` every minute as a dynamic, unprivileged user; systemd reads
the root-only settings file and keeps the probe's state in
`/var/lib/private/omoba-probe/ru/state.json`.

To see an alert reach the chat without touching the timer's state, run two rounds by
hand with a portal that cannot answer; the second one alerts:

```sh
mkdir -m 0700 /root/probe-test && set -a && . /etc/omoba-probe/ru.env && set +a
for round in 1 2; do OMOBA_PROBE_PORTAL=https://invalid.example \
    python3 /opt/omoba-probe/market_probe.py run --state /root/probe-test; done
rm -r /root/probe-test
```

## Settings

| Key | Meaning |
| --- | --- |
| `OMOBA_PROBE_MARKET` | the market key, used in messages and the unit instance |
| `OMOBA_PROBE_LOBBY` | lobby `host:port` (UDP) |
| `OMOBA_PROBE_ACCOUNT_API` | Account API `https://` origin |
| `OMOBA_PROBE_PORTAL` | portal `https://` origin |
| `OMOBA_PROBE_FAILURES` | failed rounds in a row before an alert (default 2) |
| `OMOBA_PROBE_REPEAT_MINUTES` | reminder while down (default 60) |
| `OMOBA_TELEGRAM_BOT_TOKEN`, `OMOBA_TELEGRAM_CHAT_ID` | the owner's bot; without both, alerts stay in the journal |

## From the Mac

`python3 scripts/release.py preflight --market ru` runs the same UDP check once, next
to DNS and the HTTPS routes, and `--contract dist/v<v>-ru/compatibility.json` also
compares the server's contract with a release before it ships.
