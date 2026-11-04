# Work with OMOBA Dev

## Как не перепутать DEV и PROD

**Для ежедневной работы открывай схему `Omoba Dev`.** Она собирает отдельное
приложение **OMOBA Dev** (`space.ekza.omoba.dev`) с иконкой DEV. Оно устанавливается
рядом с **Omoba Beta** (`space.ekza.omoba.beta`); у каждого своя карточка в App Store
Connect и свой TestFlight. Для тестирования на телефоне открывай OMOBA Dev.

| Что выбираешь | Что это определяет |
| --- | --- |
| Ветка и конкретный Git-коммит | Версию исходного кода |
| Схема `Omoba Dev` / `Omoba` | Идентификатор приложения и контур DEV / PROD |
| Run / Archive | Запуск на устройстве / архив для распространения |

В рабочей копии `dev` или feature-ветки открой `mobile/ios/Omoba.xcodeproj`, выбери
**Omoba Dev**, затем iPhone и Run. Для TestFlight выбери **Any iOS Device (arm64)**
и **Product → Archive**. Из терминала: `python3 scripts/release.py build ios --contour dev`.
Оба пути используют dev-лобби `lobby.dev.omoba.io:4100` и registry
`https://registry.dev.ekza.io`; dev-маркет — `https://app.dev.omoba.io/market`.
По умолчанию оставь профиль **Dev**; Local и Custom нужны только для осознанных
локальных экспериментов. Постоянно переключать ветки или серверы не требуется.

Для production используй отдельную рабочую копию релизного коммита. После
согласования релиза и его интеграции в `main` обнови ссылки командой
`git fetch origin main --tags`, выбери согласованный стабильный тег `vMAJOR.MINOR.PATCH`
через `git switch --detach <тег>` и проверь чистоту `git status`. Затем схема
**Omoba → Archive** либо `python3 scripts/release.py build ios --contour prod`
подготовит production-архив. Загрузка — отдельный явный шаг после проверки архива.
Не создавай тег ради обхода ошибки сборки.

Production Archive теперь останавливается **до Cargo**, если текущая ветка не
`main` (допускается detached HEAD), есть незакоммиченные изменения или неигнорируемые
новые файлы, на HEAD нет точного стабильного тега или коммит не входит в локальную
`origin/main`. Отсутствующие Git-данные также блокируют сборку, включая неподписанный
Archive. Настройки подписи `Omoba.local.xcconfig` и игнорируемые результаты сборки
допустимы. При случайном выборе схемы ошибка предложит **Omoba Dev**. Dev-сборки
допускают рабочие изменения; кандидат для тестеров должен собираться из чистого коммита.

Это защита от случайного выбора, а не криптографическое разрешение: теги и
`origin/main` локальные, они могут устареть или быть изменены. Старые checkout/теги
без этого изменения ещё не имеют защиты; сначала включи её в согласованный релиз.
Правило не запрещает production Run для разработки, оно проверяет именно Archive.
Старый `prepare_testflight.py` теперь отказывает в упаковке готового production
`.app`: текущее состояние Git не доказывает происхождение уже собранного бинарника.
Для production используй исходники и защищённую схему Xcode; dev-приложение этот
вспомогательный упаковщик по-прежнему поддерживает.

В архиве `Products/Applications/OmobaDev.app/assets/legal/XCODE-BUILD.json`
сохраняются SHA исходников, признак локальных изменений, Bundle ID, версия/номер
сборки, контур, Xcode action/configuration, путь архива (если передан Xcode),
явные параметры endpoint и хеш бинарника с данными dSYM. Для endpoint без явного
значения действует исходный код соответствующего SHA. Сохраняй также сам архив,
результат подписи и результат обработки Apple. Наличие архива или загрузка ещё
не означают доступность тестерам: дождись обработки и назначения внутренней группе,
затем отдельно проверь установку и матч на устройстве.

Use a checkout of `dev` containing the shared **Omoba Dev** scheme. A production
`main` checkout from 0.44 does not contain it. The server/operator entry point is
`ekza-registry/docs/dev-contour.md` on that repository's `dev` branch.

## Xcode: Run or Archive

1. Open `mobile/ios/Omoba.xcodeproj` and select **Omoba Dev**.
2. Select a physical iPhone for Run, or **Any iOS Device (arm64)** for Archive.
   The Xcode project targets devices; the Simulator uses its separate builder.
3. Keep the existing Apple team and the next unused build number in the ignored
   `mobile/ios/Omoba.local.xcconfig`:

   ```xcconfig
   DEVELOPMENT_TEAM = YOUR_EXISTING_TEAM_ID
   CURRENT_PROJECT_VERSION = NEXT_UNUSED_NUMBER
   ```

   Use a numeric build number. Do not copy credentials or provisioning profiles
   into Git. Do not set `OMOBA_CONTOUR` here: the scheme selects it.
4. Run builds `Debug Dev`; Product → Archive builds `Release Dev`. Both make
   **OMOBA Dev**, bundle ID `space.ekza.omoba.dev`, DEV icon, and default Dev
   connection profile. Local server settings do not redirect this scheme.
5. On first launch, verify DEV and the Dev profile. The lobby is
   `lobby.dev.omoba.io:4100`, the registry is `https://registry.dev.ekza.io`,
   and the market is `https://app.dev.omoba.io/market`.

The existing **Omoba** scheme builds the production app. Both apps install side
by side. The dev app's Connection panel offers Dev, Local and Custom; changes
apply after restarting. Local on an iPhone means the phone itself: use Custom
with the Mac's reachable LAN address for a server on the Mac.

Xcode configurations default to the optimized Cargo `dev` profile for iteration.
The release script uses Cargo `release`. All Xcode builds reuse the primary
checkout's `target` cache, including linked worktrees. No build should run on the
dev VPS. Keep the archive and matching dSYM for crash reports.

## TestFlight: one-time Apple setup

Register the explicit App ID `space.ekza.omoba.dev` in the existing Apple team,
then create a separate App Store Connect app record (iOS, **OMOBA Dev**, that
bundle ID, SKU such as `omoba-dev`). Do not reuse the Omoba Beta app record.
App Store Connect must have the record before upload. Xcode automatic signing
must have an eligible profile for the dev ID; creating profiles or changing
credentials is an explicit owner action.

Build and upload commands, from this checkout:

```sh
python3 scripts/release.py build ios --contour dev --dry-run
python3 scripts/release.py build ios --contour dev
# Only after the owner authorizes the upload:
python3 scripts/release.py build ios --contour dev --upload
```

The signed build command uses automatic provisioning and exports an IPA; run it
after Apple setup is authorized. An existing signed archive can instead be
uploaded from Xcode Organizer → Distribute App → App Store Connect.

After Apple processes the upload, assign it to a TestFlight group. Internal
testers are App Store Connect team users. Community testers need external
testing and any required beta review; an internal-only export cannot serve an
external group. No public App Store release is required.

Track these states separately: **built → signed → uploaded → processed → assigned
to testers → installed → tested**. An archive or successful upload alone does
not prove that testers can install or play.

## First test session

Use two desktop/iPhone clients and separate seeded accounts. The owner supplies
each account privately; account emails are `tester-NN@omoba-dev.invalid`.
These addresses cannot receive recovery mail, even after SMTP is configured.

1. Check DEV and the running Dev connection profile on each client.
2. Sign in at `app.dev.omoba.io/market`, claim a free item and connect the game
   through `hub.dev.ekza.io/studio`. Verify that the library updates.
3. Join the same match. Check that the other client sees the selected item.
4. Exercise disconnect and resume; complete the match, then join another one.
   Record logs, version/build, source revision and device/OS, never credentials.
5. Check the dev server logs for `Social send failed`, worker exits and OOMs.

The current dev lobby supports **one match at a time**. A public round has no
Leave button; bots finish an abandoned round while the lobby stays occupied.
Coordinate the first session with the operator. Empty-match cleanup is a
follow-up, not a capability provided by this Xcode change.

## Release and environment gates

As observed on 2026-10-14, the old desktop draft `v0.44.0-dev.1` targets
`1f4ad10`, before connection profiles and the new mobile app identity. Do not
use it to accept those features. Prepare a new draft from a frozen, verified
dev commit, record its SHA and checksums, and explicitly authorize distribution.

Before distribution, save the candidate's `compatibility.json` and probe the
live lobby with explicit dev arguments:

```sh
COMPATIBILITY_MANIFEST=dist/v0.44.0-dev/compatibility.json \
GAME_SERVER_ADDR=lobby.dev.omoba.io:4100 \
CARGO_TARGET_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/../target" \
make compatibility-check
```

Use the manifest next to the candidate being distributed. A passing contract probe does not
replace the full two-client match above. The installed lobby was `1ddd4d9`;
newer source includes bot-level changes that are not deployed there.

Web health: in the registry checkout, run
`deploy/dev/verify-dev.sh --remote --no-device-link`. The two dev landing Vercel
projects still need production Branch Tracking changed from `main` to `dev`.
The `omoba-app-dev` portal has a manual script deployment, no Git connection.
These are separate deployment settings requiring owner approval.

References: [connection profiles](connection-profiles.md),
[iOS archive details](../mobile/ios/TESTFLIGHT.md),
[dev lobby operations](../ops/vps/dev/README.md).
