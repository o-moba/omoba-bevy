# RU ingress (Caddy, automatic HTTPS). The portal runs on this host and calls the
# Account API directly on 127.0.0.1:40550 (README.md, "Topology"), so account.omoba.ru
# serves only the game's native routes and health; every other Account API route
# answers 403 here and needs no portal key. Both proxies OVERWRITE X-Real-IP with
# the connecting client: the Account API and the portal trust it only because of
# that (OMOBA_TRUST_LOOPBACK_PROXY=1 on both).
account.omoba.ru {
    header X-Content-Type-Options nosniff
    request_body {
        max_size 64KB
    }
    @native {
        method POST
        path /v1/auth/pairings/lookup /v1/auth/pairings/approve /v1/auth/pairings/deny /v1/auth/devices/create /v1/auth/devices/status /v1/auth/devices/recover /v1/auth/devices/complete /v1/auth/social/link
    }
    @health {
        method GET
        path /health/live /health/ready
    }
    handle @native {
        reverse_proxy 127.0.0.1:40550 {
            header_up X-Real-IP {remote_host}
        }
    }
    handle @health {
        reverse_proxy 127.0.0.1:40550
    }
    handle {
        respond "Forbidden" 403
    }
}

# The player portal, and the RU download host under /downloads/ (LR-23, README.md
# "Downloads"). The browser's address reaches the Account API only through the
# X-Real-IP header: the portal forwards it on its loopback calls.
omoba.ru {
    # Only download requests are logged, for the per-platform counts (ru_downloads.py):
    # JSON, the address shortened to /24 (IPv4) or /48 (IPv6), no request headers
    # (no cookies, no user agent), rolled files kept 90 days.
    @not_downloads {
        not path /downloads/*
    }
    skip_log @not_downloads
    log {
        output file /var/log/caddy/omoba-ru-downloads.log {
            roll_size 50MiB
            roll_keep 40
            roll_keep_for 2160h
        }
        format filter {
            wrap json
            fields {
                request>headers delete
                request>remote_port delete
                request>remote_ip ip_mask 24 48
                request>client_ip ip_mask 24 48
            }
        }
    }
    # Packages that release.py publish copied to /opt/omoba-ru/downloads/<tag>/. No
    # listing; rsync's in-progress dot files stay hidden.
    handle_path /downloads/* {
        root * /opt/omoba-ru/downloads
        header {
            X-Content-Type-Options nosniff
            Cache-Control "public, max-age=86400"
        }
        @apk path *.apk
        header @apk Content-Type application/vnd.android.package-archive
        file_server {
            hide .*
        }
    }
    handle {
        reverse_proxy 127.0.0.1:3010 {
            header_up X-Real-IP {remote_host}
        }
    }
}
