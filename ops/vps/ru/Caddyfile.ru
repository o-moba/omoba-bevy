# RU ingress (Caddy, automatic HTTPS). Replace PORTAL_KEY with the portal's
# OMOBA_ACCOUNT_INGRESS_KEY. Only native game routes are public; everything else
# on the Account API needs the portal key, like the DEV ingress.
account.ru.omoba.io {
    header X-Content-Type-Options nosniff
    request_body {
        max_size 64KB
    }
    @native {
        method POST
        path /v1/auth/pairings/lookup /v1/auth/pairings/approve /v1/auth/pairings/deny /v1/auth/devices/create /v1/auth/devices/status /v1/auth/devices/recover /v1/auth/devices/complete /v1/auth/social/link
    }
    @portal {
        path /v1/*
        header X-Omoba-Portal-Key PORTAL_KEY
    }
    @health {
        method GET
        path /health/live /health/ready
    }
    handle @native {
        reverse_proxy 127.0.0.1:40550 {
            header_up X-Real-IP {remote_host}
            header_up -X-Omoba-Portal-Key
        }
    }
    handle @portal {
        reverse_proxy 127.0.0.1:40550 {
            header_up X-Real-IP {remote_host}
            header_up -X-Omoba-Portal-Key
        }
    }
    handle @health {
        reverse_proxy 127.0.0.1:40550
    }
    handle {
        respond "Forbidden" 403
    }
}

app.ru.omoba.io {
    reverse_proxy 127.0.0.1:3010 {
        header_up X-Real-IP {remote_host}
    }
}
