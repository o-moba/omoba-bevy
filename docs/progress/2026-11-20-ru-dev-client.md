# Shared RU host for dev clients

On the RU dev contour, the client uses lobby.dev.omoba.ru on UDP 4100. The allocated match worker uses UDP 41100 when both contours share the Russian host. These ports apply after DNS resolution, to the signed directory, to the static fallback and to reconnects. UDP 4000 and the production workers continue to serve the production client. Account and portal traffic uses the existing regional HTTPS origins.

The Android packager now accepts the RU dev preset with its built-in account and portal endpoints. Global dev retains the Ekza registry and passport checks.

Client tests cover both roles, mapped IPv6, regional binding, directory selection and resuming a worker. Public RU-dev DNS, service deployment and physical device checks are separate release steps.
