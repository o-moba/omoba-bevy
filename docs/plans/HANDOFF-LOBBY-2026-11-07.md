# OMOBA — передача следующему агенту, 2026-11-07

## Актуальный checkpoint — native Linux 300 s soak/A/B, 2026-11-09

[Новый soak evidence packet и verifier](../evidence/linux-capacity-soak-2026-11-09/README.md)
фиксирует два300-second loopback прогона на общем dev VPS (2 vCPU,3480344KiB RAM),
те же release server/native ten-client shards и source parent
`4a0f2ca616a678371bd4d6f2798e117e03357f57`. Runner отдельно расширен до15–300s;
13/13 Python tests PASS. Rust sources/binaries от предыдущего boundary не менялись.
Release42611040419 текущего parent полностью SUCCESS,включая macOS. Основной
CI42611040463 на момент пакета имеет6/7 SUCCESS,PostgreSQL journey ещё in_progress.
Новый documentation/evidence HEAD обязан получить собственный полный CI.
Draft release на PR не создаётся.

- **60 clients/6 Casual arenas/300s, server metrics OFF: PASS**19.5349–19.6209
  sends/s,accepted snapshots p50/p0518.4171/18.3927Hz,wire p50/p9587783/89275
  B/player/s. Server CPU72.46%,RSS299.86MiB;generator69.11%/44.66MiB.
  Server dispatch/cadence здесь явно не измерены,отсутствие LOAD_METRICS проверено.
- **Тот же профиль, metrics ON: FAIL** неизменного19.5–20.5Hz input gate:
  19.4589–19.5355 sends/s,accepted snapshots18.3411/18.3115Hz. Все60 peers и6
  exact rosters удержались300s. Из worker logs независимо пересчитаны416943
  application dispatches,356001 transforms,337456 snapshots,**0 gaps/restarts**.
  Broadcast mean54.497ms,p95/p99<75/100ms;latest-transform age26.038ms,<55/150ms.
  Server CPU72.27%,RSS300.60MiB;generator66.98%/44.70MiB. FAIL сохранён.
- Один A/B **не доказывает** причину metrics: без них server CPU выше на0.19pp,
  generator выше на2.13pp; minimum input изменился только на0.0760/s. Это
  borderline shared-host variability,не unconditional six-arena/RU acceptance.
- Median wire даёт41.711/42.136Mbit/s aggregate application traffic на60 players;
  p95-per-player×60=42.739/42.852Mbit/s. Network headers не включены,uplink/WAN
  не измерены. Histograms cumulative/bucket upper bounds,не per-command ACK SLA.
- Оба run имеют final health-v2 и полный owned process/DB/role cleanup;dev lobby/
  API active. Verifier проверяет20 selected hashed files,15-source archive,
  input/wire/population/resources/server metrics и FAIL/PASS различие.

Прежний [stepped capacity packet](../evidence/linux-capacity-2026-11-09/README.md)
не переписан:40/4 и60/6 по120s PASS,80/8 FAIL с7 gaps. Его43-file verifier и
Node A/B сохраняют вывод о bottleneck одного Node generator;для capacity нужны
native shards. `ops/vps/ru/README.md` считает16MiB/arena неподтверждённой.
**Unit,16-arena setting,nftables иproduction formula не изменены.** Реальный или
идентичный RU-host,vs Bots/mixed,uplink,WAN и SLA остаются open;#13 не закрыт.

Портал `37e705c`: typecheck,260 unit (257 PASS/3 expected skips),RU/global
production builds PASS. Ranked-seasons6 и ranked-notices7 Next+Chrome RU checks
PASS без page errors/unexpected API; локальные серверы остановлены. GitHub checks
у PR33 не настроены; leaderboard/game-directory требуют своих loopback binaries/
fixtures. В обоих репозиториях GitHub показывает только автора `wotori` как
collaborator,поэтому нужно человеческое ревью владельца/приглашённого reviewer,
не self-review.

Прежние [server-load1000 proof](../evidence/server-load-metrics-2026-11-09/README.md),
[buffered telemetry/input20 proof](../evidence/input20-buffered-2026-11-07/README.md)
и rejected diagnostics сохранены без перезаписи. Тяжёлый Cargo/Bevy/SDK build не
запускался,третий cargo cache не создавался.

**Следующий capacity gate:** реальный/идентичный изолированный RU-host,vs Bots/
mixed и measured uplink/network budget; host acceptance не должен скрывать
instrumented300s FAIL. Не повторять локальный A/B как замену production proof.
Далее WAN/multi-host/party/Ranked/Resume/fault,directory/trust provisioning;
current UI-2/UI-3 иphysical C3; independent off-host/RU RPO/RTO иC2/C4 после
существующих решений владельца; legal/provider/signing/catalog/device/release.
Сохранять полный scope68+9+UI/physical/off-host/WAN/trust/RU,исходные68=62/5/1.

Основная работа по-прежнему в feature branches `feat/ranked-matchmaking-readiness`:
игра mm-game/PR148 и портал mm-web/PR33 →dev, оба open/not merged. Ни dev/main
merge,ни release/deploy/reset/keys/grants не было. Следующие коммиты должны
получить собственный CI.

## История b3c74bd1 — timing до исправления вывода

Предыдущий goal-turn был progress: сохранён и запушен полноценный checkpoint,
не завершение всего релиза. В этом продолжении prepared v2 уже реально выполнен.
[Точные новые отчёты, диагностика и verifier](../evidence/input20-timing-2026-11-07/README.md)
приоритетнее исторического «v2 NOT RUN» ниже.

- V2:10/30s PASS20.001Hz;1000/120s **FAIL**18.771–19.583Hz, median19.196.
- V3 control PASS20.054Hz. Большой запуск остановлен preflight из-за занятого
  UDP port; серверы не стартовали, затем wrapper сообщил об отсутствующем report.
  Сохранён первичный отказ. Preflight теперь явно сообщает порт и exit2; настоящий
  occupied-socket test доказывает отказ до SQL/process startup. Чужое не трогали.
- V4 в отдельной fixture/других портах:1000/120s **FAIL**19.346–19.869Hz.
- V5 с раздельными timing:1000/120s **FAIL**18.819–19.611Hz. Все три полных
  попытки удержали Running100 арен/1000 signed AI peers, rosters совпадают,
  authority resets0. Ни одна не закрывает неизменный19.5–20.5Hz per-peer gate.
- Добавлены реальные меж-send gaps и cumulative wall histograms с вычитанием
  baseline. В V5 loop p99<150ms, receive/session/navigation p99<1ms;3966 overrun
  rebases, send-gap p95<75ms/p99<150ms. Это bucket bounds, не точные percentiles;
  wall включает scheduling. Bulk stalls не доказаны как navigation issue.
- Один проверенный собственный generator54 sampled3s/1ms вV5, call graph сохранён.
  Это дополнительный диагностический overhead; не выдавать сравнение V5/V4 за
  контрольный performance experiment. Most samples under sleep/receive/decode,
  меньшая часть signed send. Следующим отдельно измерить serialization/signing,
  syscall send и stdout/telemetry. Сначала короткий явно diagnostic interval на
  полном населении для причины, после реального фикса снова120s acceptance.
-38 whole-harness +4 UDP +25 Python и4 strict Clippy PASS, включая real signed
  gap/Ping/reset и backwards/overflow histograms. Production server unchanged
  SHA5c6f4e6f…; opt1 manifest содержит248 non-harness Rust sources (проверены),
  а263 total включали15 harness, не все были production. Gameplay/authority/
  timers/schema/dependencies/CI/RU configuration не менялись.
- Все свои v2/v3/v4/v5 DB/volume удалены, финальные process inventories пусты.
  **Живых локальных probes нет.** Helpers/build manifests/retained binaries:
  `.agent/codex-ranked-readiness/load-input20/`. Native v2→v4→v5 имеют разные
  hashes; не использовать отчёт от другого бинарника. Root≈12GiB: no heavy build.
- Parent de5291fd CI42357470179 пока6/7 SUCCESS, PG in_progress; Release
  42357470196 cancelled: macOS cancelled, Windows/Android/Linux success,
  draft-release skipped. После push найти новый head/run, не перезапускать живые
  jobs из-за observation timeout и не считать parent полным gate текущего пакета.

Scope **68+9+UI/physical/off-host/WAN/trust/RU** сохраняется. Исходные68 всё ещё
62done/5partial/1open, C2/C4 review-only с прежними вопросами владельцу; ни merge,
ни production deploy/reset/keys/grants не было. Список6 PR и все решения ниже.

## История передачи de5291fd — до live v2

Владелец попросил сохранить весь контекст для переключения агента. Этот блок
приоритетнее исторических обновлений ниже. [Новый проверяемый пакет](../evidence/input20-handoff-2026-11-07/README.md)
содержит точные сжатые отчёты, два разных build manifests, verifier и копии
контроллера/сборочного helper. Текущий checkpoint сохраняет все шесть изменённых
harness/scripts файлов; production server/store/shared/SDK не менялись.

- Реальная отправка20Hz теперь измеряется по успешно отправленным signed UDP
  Transform; не по requested flag и не по поступлению snapshots. Есть отдельный
  счётчик использования cached snapshots; receive freshness он не обновляет.
- Первый blocking v1:10 клиентов/30s PASS,20.020Hz у каждого.1000 клиентов/120s
  удержались Running, но весь сценарий **FAIL**:18.163–19.300Hz, median18.780,
  ниже строгих19.5–20.5. Все исходные FAIL/метрики сохранены. Authority resets0,
  rosters100/1000, cleanup полный. Не считать это20Hz capacity PASS.
- Исправление v2: bounded nonblocking socket drain вместо1ms ожидания на каждого
  peer.36 library +4 real-UDP +23 Python +5 invalid CLI и4 strict Clippy PASS;
  текущие исходники/3 binaries сверены с manifest. **V2 live ещё НЕ ЗАПУЩЕН**.
- Никаких живых локальных build/load процессов этого пакета не осталось.
  `.agent/codex-ranked-readiness/load-input20/rehearse_v2.py` подготовлен;
  `live-v2` ещё не существует. Перед запуском заново проверить это состояние.
- Следующий шаг: новая lease, проверка Git/process/source hashes, затем
  `/opt/homebrew/bin/python3.13 .agent/codex-ranked-readiness/load-input20/rehearse_v2.py`
  из игрового worktree. Он последовательно запускает10/30s и1000/120s в своей
  PG18.6/schema13/11 fixture. Подробности/guardrails в новом README. Не менять
  acceptance tolerance и реальные consent/auth/game/recovery таймеры.
- При успехе дальше send gaps/per-command acceptance/input-to-snapshot latency,
  duration/Linux/RU/uplink/WAN. Иначе исправлять подтверждённую причину и сохранять
  точный отказ. Остальные UI/physical/off-host/trust/release gates не потеряны.
- Parent f0 CI42354938539 на момент передачи:6/7 jobs SUCCESS, PostgreSQL-backed
  ещё in_progress. После push нового checkpoint найти его собственный run;
  parent green/живой job не является полным gate новых harness файлов.
- Через GitHub API заново подтверждены **ровно6 открытых PR** двух репозиториев,
  ссылки/ветки/draft приведены ниже. Все6 присутствуют во вкладках IAB. Ничего
  не merge/release/deploy. Портал остался f868fa4641d74899d5c5844def5fd078281fd9a7.

**Не потерять:** исходные68 =62 done/5 partial/1 open; дополнительно9 пунктов
критика и UI/physical/RU. C2/C4 остаются review-only с существующими вопросами
владельцу. Системный диск около12GiB: тяжёлый Bevy/SDK build запрещён; общий
cache B только read-only для этого маленького headless harness. Unlock Mac
не просить: offscreen recipe уже сохранён в репозиторном skill.

**Обновление после30f69c5e:** [current opt1 local population](../evidence/optimized-playing-1000-2026-11-07/README.md)
PASS:1000 signed AI peers/100 ten-seat Casual arenas/120s Running,0 server bots,
0 observed auth resets;100 final health-v2 workers SQL/journal ready, no backlog.
Тот же production source (263 hashes), но явный renderer-free workspace opt1
с debug/overflow checks; dependencies read-only, без Cargo/SDK/Bevy build.
Новое75.65MiB, root min12.24GiB. Все Running44.96s, received median12.726Hz/p05
11.969, p95 wire74123B/player/s, total received485.77Mbit/s без IP/UDP overhead.
Server3.644 cores/1694.67MiB peak summed RSS; generator2.764/625.64MiB.
Это не full performance/RU PASS: native AI спит100ms и посылает transform после
полученного snapshot; client update interval50ms. **Следующим дать измеренный20Hz
input без привязки к поступлению snapshots и повторить полный target**, затем
cadence/latency и Linux/RU/uplink/WAN. Этот прогон не покрывает20Hz movement input.
Бывшие unoptimized failures остаются своими FAIL.
Shared snapshot_tick резервируется ещё и на bootstrap1Hz до фильтрации игроков:
accepted/sequence fraction — НЕ packet delivery/loss. Добавлены scope caveats в
runner; старые значения/архивы неизменны. No zero-loss claim.
Reusable opt1 server/bots + exact scripts/raw data:
`.agent/codex-ranked-readiness/load-opt1/`; build.json содержит flags/хеши.
Контроллер теперь ждёт стабильного post-exit inventory, сохраняет audit при
отказе и не сигналит transient `(server)` labels. Final inventory пуст, свой
DB/volume удалён. Другие scopes68+9+UI/physical/WAN/RU ниже сохраняются62/5/1.

**Обновление после865e27be:** исправлены две подтверждённые ошибки native AI
probe:20ms receive deadline фактически блокировался до200ms; Career envelopes
не имели native epoch/sequence fence. Отрицательные UDP/authority tests
воспроизводят ошибки,35 library +3 UDP +22 Python и4 strict Clippy PASS.
[Новый пакет и полные границы](../evidence/load-probe-deadlines-2026-11-07/README.md).
Два последующих1000-прогона **FAIL до soak** (106.26/108.35s);10-client controls
PASS. Последний срез даёт0 stale Career packets и209 реальных authority resets:
не объявлять reordered Career причиной этого отказа. Изучать auth refresh/expiry
и transport transitions под нагрузкой; для performance нужен явно optimized
current-source/coherent graph. Не увеличивать auth TTL и не ослаблять проверки.
Latest bots SHA40006f16…, server7a736997…/263 production sources неизменны.
Raw `.agent/codex-ranked-readiness/load-deadline` и `load-career-order`; helpers
и negative tests в `load-cadence`. Оба DB/volume удалены, групп процессов нет.
Контроллеры сохранены как история: при cleanup кратко увидели непроверяемое
имя `(server)`/stateS и отказались сигналить. Перед повтором исправить ожидание
стабильного inventory/обязательное сохранение cleanup audit, использовать новые
имена/output. Это не доказанная причина admission failure. Полный scope ниже
сохраняется, исходные68 всё ещё62/5/1. UI/physical/RU gates не закрыты.

**Следующее обновление после efe32101:** текущий1000-client прогон **FAIL до
начала soak**, а не очередной capacity PASS.10/100 контроль PASS; добавлены
per-peer producer ticks, сохранение baseline и first-owned-exit diagnostics.
Также исправлен stop_owned: PermissionError на одной группе не прерывает очистку
остальных; непроверенная очистка группы остаётся явной ошибкой.14 дополнительных
Python cases PASS, в том числе два real-child lifecycle regression.86 оставшихся
своих процессов остановлены после проверки UID/пути/группы, финальный inventory
групп пуст. Это отдельный cleanup bug, не установленная причина admission failure.
[Полный пакет, границы и следующий шаг](../evidence/load-cadence-2026-11-07/README.md).
Следующим повтором определить первый сбой до teardown; это новая актуальная
проверка прежних frozen нагрузочных выводов. Существующий native server —
direct-rustc/root opt0, не release benchmark; для performance нужен явно
зафиксированный optimized build. Не менять gameplay/таймеры ради зелёного теста.
UI-2/UI-3 всё ещё ждут current pixels: готовый e70b6657 binary старше кандидата,
root13GiB ниже20GiB, поэтому новую тяжёлую сборку не запускали. Mac unlock не нужен.

**Обновление после checkpoint a3ba5f4e:** локальный PG16 terminal-file replay
и настоящая повторная доставка одинаковых bytes теперь проверены:22 стадии PASS.
[Актуальные отчёты и полный verifier](../evidence/terminal-spool-replay-2026-11-07/README.md).
Старый прототип не применять повторно. Следующие области — текущий CI, UI/physical,
контролируемая нагрузка/WAN/trust и независимое off-host/RU восстановление с
согласованным C2/C4 kit. Исходный scope и решения ниже сохраняются.

## Начать здесь

Продолжить исходную задачу владельца: **закрыть все68 находок,9 пунктов критика
и довести OMOBA до готовности к релизу в России**. Не сужать до «код написан»
или «локальные тесты зелёные». Обновлять TODO/changelog и сохранять небольшие
проверенные коммиты с push. Этот документ входит в очередной checkpoint поверх
`9ed77bd819084ebfbb214753490f6b9b23d692c5`; актуальный HEAD получить из Git/PR.

Прочитать по порядку:

1. [Полный TODO, текущий порядок и68 строк](TODO-LOBBY-2026-11-05.md).
2. [Progress](../progress/2026-11-05-ranked-matchmaking-readiness.md) и
   [changelog](../../CHANGELOG.md).
3. [Текущие PG16/18 отчёты восстановления](../evidence/worker-physical-recovery-2026-11-07/README.md).
4. [C4: актуальный кандидат](reviews/C4-PITR-PARAMETERS-2026-11-07.md) и
   [C2: кандидат drain/update/rollback](reviews/C2-DRAIN-2026-11-07.md).
5. `.claude/skills/omoba-ui-verification/SKILL.md` и её рецепт offscreen.

Исходные68: **62 done,5 partial (#1,#2,#12,#20,#50),1 open (#13)**.
Критик, UI, физические устройства и RU-выпуск — дополнительные условия.
Их не считать закрытыми из-за этого счётчика. Оценка3–7 рабочих дней была
предварительной и зависела от устройств, инфраструктуры и согласований;
это не обещанная дата выпуска.

## Репозитории и все открытые PR

Рабочий umbrella: `/Users/wotori/git/ekza`, сам umbrella не Git-репозиторий.

| Назначение | Worktree / ветка | PR / состояние на момент передачи |
|---|---|---|
| Игра, сервер, Account API, ops | `_workspace/worktrees/mm-game`, `feat/ranked-matchmaking-readiness` | [omoba-bevy#148](https://github.com/o-moba/omoba-bevy/pull/148), open, в `dev`; сюда сохраняется этот checkpoint |
| Портал | `_workspace/worktrees/mm-web`, та же ветка; HEAD `37e705c04acb59816710584c414619b80f687f35` | [omoba-web#33](https://github.com/o-moba/omoba-web/pull/33), open, в `dev` |
| Выпуск игры | `dev` → `main` | [omoba-bevy#101](https://github.com/o-moba/omoba-bevy/pull/101), **Draft**, решение владельца |
| Выпуск портала | `dev` → `main` | [omoba-web#10](https://github.com/o-moba/omoba-web/pull/10), **Draft**, решение владельца |
| Документация RU offer | `docs/lr-67c-built` → `dev` | [omoba-bevy#146](https://github.com/o-moba/omoba-bevy/pull/146), open, отдельная более ранняя работа |
| Документация портала/магазина | `docs/ru-web-deploy-20261105` → `dev` | [omoba-web#30](https://github.com/o-moba/omoba-web/pull/30), open, отдельная более ранняя работа |

Это полный проверенный список открытых PR в двух репозиториях OMOBA, а не список
всех PR организации. Все шесть открыты во вкладках для владельца. Release/Draft
и сторонние docs PR не сливались и не редактировались в этой передаче.
PR101/10 пока содержат dev, а feature148/33 в него ещё не влиты; после интеграции
проверить и обновить release descriptions/контракты, не полагаясь на их старые версии.
PR30 описывает прежнюю выкладку портала другим этапом; это не означает выкладку
текущих PR148/33. Состояние живого сервера заново не проверялось и не менялось.

Замороженный `_workspace/worktrees/mm-p6-verification` на `0db576fc` не трогать:
это историческое доказательство. Старые записи/скриншоты имеют собственные SHA
и область проверки; нельзя переименовать их в current PASS.

## Уже реализовано и подтверждено

- Все люди подтверждают матч за15 секунд **до создания worker**. При пропуске
  начинается свежий поиск/подтверждение; невиновные сохраняют группу и FIFO.
  Подтверждения, владение и cooldown за dodge хранятся в SQL между лобби.
- Ranked после10 завершённых публичных матчей, без обязательного VK. Неигранные
  отмены не дают сыгранного матча. AFK/leave, remake, surrender, личная санкция
  победителя и нейтральный результат Interrupted/Remade реализованы и проверены
  в своих зафиксированных областях.
- Уникальные классы внутри команды, один публичный выбор героя, дружеская группа,
  повторный поиск и возврат после сбоев, сезонные карточки/история/уведомления.
- 20 калибровок,6 рангов,90-дневные сезоны; архив/значки и административный сброс.
  Runtime не вправе закрывать/архивировать/сбрасывать сезоны.
- Протокол16 со сжатием; signed directory/native routing/cache/trust реализованы.
  Реальная публичная упаковка доверия, ключи/публикация и WAN ещё не приняты.
- Runtime drain сохраняет живые матчи/Resume. Изолированные процессы доказывают
  adoption после рестарта лобби и SIGTERM с durable outbox.
- Schema13: bounded ready retention, обязательное SQL-согласие при новом
  public admission; отсутствие SQL authority при восстановлении не превращается
  в обход проверки. Файлы с результатами сохраняются до восстановления authority.
- Исправлены starvation reconnect и блокирующие Ready/Flush replies. Критичная
  очередь до256, резерв4, pause intake с10ms async yield; SQL/leases/health живы.
  Сохраняются точные sequence/result/flush generation. Health operational format2
  показывает backlog и известное закрытие получателя. Producer/monitor выкатывать
  согласованно после drain; schema/protocol этим форматом не менялись.

Текущие версии: **career13 / portal11 / protocol16 / operational health2**.
Миграция013, узкие runtime grants и binaries должны соответствовать друг другу.
Schema12 binary не является rollback поверх13.

## Проверки и их точный смысл

| Доказательство | Результат / ограничение |
|---|---|
| [Critical replies](../evidence/critical-replies-2026-11-07/README.md) |575 server cases, включая16 реальных PG;101 store cases для неизменных Rust/SQL;42 monitor и74 host-tool. Не складывать повторяющиеся suites в новый общий счётчик |
| [C4 revised candidate](../evidence/pitr-recovery-limits-2026-11-07/README.md) |177 cases:176 pass,1 optional skip. Только изолированный кандидат, не применённый host kit |
| [PG18 current worker](../evidence/worker-physical-recovery-2026-11-07/pg18/summary.json) |22 стадии, физическое восстановление, настоящий public bot-practice worker, SCRAM/actual grants, тот же игрок/epoch/worker Resume, adoption, SIGKILL,223.47s, один Interrupted без Elo/XP |
| [PG16 current worker](../evidence/worker-physical-recovery-2026-11-07/pg16/summary.json) |Те же22 стадии,221.63s; ждать writable promotion после connection readiness; все свои контейнеры/сеть удалены |
| [Полная signed PG16 игра](../evidence/worker-physical-recovery-2026-11-07/pg16/runtime/report.json) |Один protocol-client и9 обычных server bots, **не**10 Ranked людей, не физический телефон/WAN |
|1000-playing frozen78070b27 |100 арен,1000 signed AI peers,60s; received median8.64Hz иp95≈39347B/player/s. Не sustained SLA и не вместимость RU |

PG16/18 source limits80/12/14/4/128 выявили дефект исходного C4: восстановление
жёстко ставило20 connections. Актуальный **review-only** кандидат берёт валидированные
числовые maxima из primary и pg_controldata для пяти размеров shared memory.
Изначальный C4 patch и старые20-connection/schema10 отчёты сохранены как история.
C2 patch после нового C4 применяется; это application check, не новое combined
поведенческое подтверждение C2 и не согласование владельца.

У обоих прежних live crash прогонов перед падением было **0 JSON в outbox**.
Их собственная область — SQL/checkpoint recovery. В новом отдельном PG16 прогоне
настоящий SIGTERM при SQL offline оставляет Settle, отсутствующий в source SQL;
после physical restore автоматический worker и затем второй public worker ACK-ают
те же bytes. Полный результат совпадает кроме saved, credit один, Elo/XP нет.
Это закрывает локальную проверку terminal-file, но не independent-storage/RU readiness.
В rejected-attempts прежних прогонов явно сохранены ошибки
подготовки:20 против80, внутренний Docker network без published port,
пропущенное сериализацией `true`, PG16 read-only readiness. Они не PASS.

## CI: перепроверить, а не перезапускать вслепую

- `a654f032` CI [42343372154](https://github.com/o-moba/omoba-bevy/actions/runs/42343372154)
  **SUCCESS, все7 jobs**. Artifact13563604431,535187B,
  SHA256 `8c278653a3370cc490f53c50f8fe3c897181445aa55d3b6777e89d2a25c4a318`:
  ZIP/CRC/пути и все20 public stages проверены, private fixture удалена.
  [Зафиксированная проверка](../evidence/worker-physical-recovery-2026-11-07/parent-ci.json).
  Это полный CI критичной очереди/health2, не автоматический PASS более новых
  файлов тестового сценария и передачи.
- На снимке `9ed77bd8` CI [42346476996](https://github.com/o-moba/omoba-bevy/actions/runs/42346476996)
  ещё in progress; `f8510b57` [42344043379](https://github.com/o-moba/omoba-bevy/actions/runs/42344043379)
  тоже. После push этого checkpoint будет новый run. Получить актуальный head/run
  из PR/GitHub. Remote CI продолжает работать после закрытия чата.
- Release workflow на ряде предыдущих PR-head отменял macOS job. Это отдельный
  незакрытый platform gate. Причину на новом head проверить; не считать cancel
  успешным тестом. Dispatch Release может создавать draft release: без владельца
  не запускать публикацию. Наблюдательный timeout не означает, что job остановлен.

## Что делать следующим

1. Проверить текущие Git status, PR head и конкретные CI handles. Сохранить
   результаты текущего полного прогона и исправить подтверждённые ошибки.
2. Локальный terminal spool уже принят: [22 PG16 стадии](../evidence/terminal-spool-replay-2026-11-07/README.md),
   реальный SIGTERM/SQL offline, исходный файл не staged в SQL, physical restore
   matching admission, точная повторная доставка вторым public worker, ACK обоих
   файлов и один credit. Полный raw payload отличается только saved=false→true.
   Старый [TERMINAL-SPOOL-PROTOTYPE](reviews/TERMINAL-SPOOL-PROTOTYPE-2026-11-07.patch)
   — история; не применять его поверх уже готового scripts/test_public_mvp.mjs.
3. Независимое off-host/RU восстановление и RPO/RTO ещё не приняты. C4/C2 пока
   proposals, не установленные на сервер. Дождаться существующих решений на
   интеграцию/выкладку и подготовить независимое storage/escrow/recovery испытание.
   Это не блокирует независимые UI/physical и controlled-load/WAN/trust задачи.
4. UI-2 widest names и UI-3 compatibility на точном текущем native build;
   C3 physical keep-awake/haptics. Для заблокированного Mac читать offscreen skill,
   не просить снова unlock. Offscreen не доказывает физические устройства.
5. Sustained текущая нагрузка/cadence/latency, реальный/идентичный RU host и uplink,
   multi-host/WAN party/Resume/faults, public trust packaging/pins/publication.
   Оригинальные#1/#2/#12/#20/#50/#13 остаются в полном объёме.
6. После существующих согласований — интеграция кандидатов C4/C2, нормальный CI,
   независимое off-host/RU восстановление/RPO/RTO, проверенная выкладка/откат.
   Финальный RU release также сохраняет real-provider/legal/signing/catalog/device
   gates из launch/readiness и draft release PR101/10. Их не отменяет этот TODO.

Не сокращать реальные recovery/consent/lease таймеры ради зелёного теста.
Новый слой проверки должен доказывать свою область: SQL receipt, UI pixels,
physical device, WAN и RU host не подменяют друг друга.

## Решения и уже выданные разрешения владельца

Не спрашивать их заново:

- 15s unanimous ready; fresh search после пропуска, FIFO/группа невиновных сохранены.
- Ranked после10 completed public; VK не обязателен. Это заменяет старый O-3
  «no ranked entry gate» в launch-readiness.
- 20 калибровок; Bronze≤999, Silver1000–1499, Gold1500–1999,
  Platinum2000–2499, Diamond2500–2999, Master≥3000.90 дней;
  reset `1000 + половина разницы со старым`; архив/сезонный значок.
  Применение реального reset — отдельное действие.
- Общая очередь touch/mouse, собирать статистику.
- Android VIBRATE и прямой jni0.21.1; flate2 из закреплённого графа;
  patch прав сезонов; CI full-game/диагностика/90min уже согласованы.
- Промежуточные коммиты после целевых локальных проверок, полный gate в CI
  из-за системного диска<20GB; cache на `/Volumes/Orico/` уже указан владельцем.

**Остаются существующие решения:** C4 production dependency/host kit и C2
deploy/drain/rollback tooling. Они пока review-only. Для C4 использовать новую
параметрическую ревизию, не устаревший hardcoded20 patch. Эти вопросы не дают
права устанавливать пакет, менять secrets/archive config, рестартовать RU,
применять миграции/grants/reset, main/release или production deployment.
Обычный код, тесты и исправление прототипа terminal replay внутри задачи уже
разрешены; отдельного разрешения на такой test-only patch не требуется.

## Локальные инструменты и ограничения

- Перед любой записью читать umbrella `_workspace/skills/cross-agent-lock/SKILL.md`,
  получить **свою новую** atomic lease на нужные пути. Чужую не ломать.
  Текущая lease будет освобождена при завершении передачи; её handle не переносить.
- Один текущий Codex; параллельных subagents на этом этапе не запускали.
  Не запускать новых без явного запроса или применимых инструкций.
- Системный диск около13GiB free: **никаких тяжёлых Bevy/SDK builds**, cargo clean,
  удаления общих кэшей/чужих файлов. Только два разрешённых shared cache, A/B;
  назначен B, фактический путь `/Volumes/Orico/omoba-build-cache/target-b`.
  Не создавать третий или worktree target. Проверять `df -h /` перед тяжёлым build.
- Все нижние команды выполнять из игрового worktree. Использовать
  `/opt/homebrew/bin/python3.13` — системный python не имеет tomllib.
- `.agent/codex-ranked-readiness/ready-runtime/`: проверенный `server`,
  `server-tests`, `server-compile.json`, `link_server.py`, `check_server_clippy.py`,
  `run_server_pg.py`, `run_probe.py`. Это ignored local helper/artifacts,
  сохраняются на этом Mac, но не заменяют переносимые tracked отчёты.
- Native production server SHA256:
  `7a73699731633f1a4d0831ec41715e789d91479649fadd0cb343c5dce2dea1a9`.
  Раннер сверяет binary, actual production sources и helper hashes. При изменении
  источников старый бинарник больше не подтверждает исправление.
- `link_server.py tests|server` связывает **действительный текущий** root crate
  с согласованным read-only cached dependency graph, без компиляции graphics/SDK.
  Production common не содержит test-support. Не менять граф/rlibs параллельно
  с линковкой. Full engine/client gate остаётся CI.
- `run_server_pg.py [filter]`: собственный `omoba-mm-seasons-pg`, localhost55582,
  database omoba_test, URL из Docker env только в окружение; никогда не печатать
  пароль/DSN. Нативные PG tests — `--include-ignored --test-threads=1`.
- `run_probe.py scenario suffix` занимает46500/46600. Current physical runner
  занимает46530/46630. Запуски последовательны; не атаковать чужие процессы.
- Review helper copy для физического раннера:
  `.agent/codex-ranked-readiness/pitr-parameters/current-tree/ops/vps/ru/`.
  Изображения `omoba-mm-pitr-kit-rehearsal:20261107` и
  `omoba-mm-pitr-kit16-rehearsal:20261107` уже есть; не скачивать/строить заново
  без необходимости. Все временные pitr containers/networks текущих прогонов удалены.
- `.agent/codex-ranked-readiness/terminal-spool/probe.mjs` — старый черновик,
  не текущий сценарий. Проверенный вариант теперь в scripts/test_public_mvp.mjs,
  контроллер в docs/evidence/terminal-spool-replay-2026-11-07/rehearse_terminal_replay.py.
  Raw accepted local run: `.agent/codex-ranked-readiness/terminal-spool/live-v1`.
  Проверка payload: python3 docs/evidence/terminal-spool-replay-2026-11-07/verify_payload.py.
  Она сравнивает committed данные; не выдавать её за новый live run.
- Не делать `git add .agent` целиком: другие fixture/state могут содержать ключи.
  Публиковать только проверенные обезличенные evidence; новые evidence dirs
  игнорируются Git, добавлять выборочно `git add -f docs/evidence/<packet>`.
- Перед любыми dev VPS/domain/Vercel действиями читать `ekza-registry/docs/dev-contour.md`
  на `dev`. Никакого production из dev-работы. main/releases/production — владелец.

## Готовая формулировка для следующего агента

Продолжи OMOBA по этому HANDOFF и полному TODO. Проверь актуальные Git/CI,
затем бери первый оставшийся gate. Локальные actual terminal-file/duplicate PG16
и SQL/checkpoint/Resume PG16/18 уже приняты; не повторяй их без причины.
Сохрани полный scope68+9+UI/physical/WAN/RU,
уже принятые решения и существующие C2/C4 согласования. Обновляй TODO/changelog,
коммить/push отдельными пакетами. Не проси повторять уже выданные разрешения.
