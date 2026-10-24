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

# The player portal. The browser's address reaches the Account API only through
# this header: the portal forwards it on its loopback calls.
omoba.ru {
    reverse_proxy 127.0.0.1:3010 {
        header_up X-Real-IP {remote_host}
    }
}
