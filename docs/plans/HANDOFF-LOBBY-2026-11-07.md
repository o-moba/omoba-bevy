# OMOBA — передача следующему агенту, 2026-11-07

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
| Портал | `_workspace/worktrees/mm-web`, та же ветка; HEAD `f868fa4641d74899d5c5844def5fd078281fd9a7` | [omoba-web#33](https://github.com/o-moba/omoba-web/pull/33), open, в `dev` |
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

У обоих принятых live crash прогонов перед падением было **0 JSON в outbox**.
Следовательно, доказано SQL/checkpoint recovery, но **не terminal-file replay**.
Это следующий обязательный шаг. В rejected-attempts явно сохранены ошибки
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
2. **Terminal spool через физическое восстановление**. Непроверенный прототип
   [TERMINAL-SPOOL-PROTOTYPE](reviews/TERMINAL-SPOOL-PROTOTYPE-2026-11-07.patch)
   не применён к scripts/test_public_mvp.mjs и не является PASS. Синтаксис проверен.
   В нём ещё нет настоящего duplicate redelivery: повторный Profile read его
   не заменяет. Не маркировать готовым по этому шаблону.
3. Контроллер сценария ещё дописать: настоящий матч → physical backup/архив →
   закрыть только свой DB proxy → SIGTERM своего worker → увидеть реальный
   `Settle` JSON и `SHUTDOWN_METRIC terminal_outbox=durable` при недоступной SQL →
   убедиться, что source SQL ещё running/intent отсутствует → physical restore
   той же admission → дождаться автоматического recovery child и ACK файла.
   Сравнить итоговые ended_at_ms/duration/stats с **реальным** terminal JSON,
   иначе checkpoint fallback может ошибочно выглядеть как terminal replay.
   Затем действительно повторно доставить точные bytes, доказать неизменный
   один career credit и освобождение назначения. Ничего не подделывать в SQL.
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
- Незавершённая local копия test-only terminal probe:
  `.agent/codex-ranked-readiness/terminal-spool/probe.mjs` и socket helper.
  Это только черновик, контроллер offline/restore и actual duplicate replay ещё
  не дописаны/не выполнены. Проверенный вариант сохранить отдельным checkpoint.
- Не делать `git add .agent` целиком: другие fixture/state могут содержать ключи.
  Публиковать только проверенные обезличенные evidence; новые evidence dirs
  игнорируются Git, добавлять выборочно `git add -f docs/evidence/<packet>`.
- Перед любыми dev VPS/domain/Vercel действиями читать `ekza-registry/docs/dev-contour.md`
  на `dev`. Никакого production из dev-работы. main/releases/production — владелец.

## Готовая формулировка для следующего агента

Продолжи OMOBA по этому HANDOFF и полному TODO. Проверь актуальные Git/CI,
затем закончи actual terminal-file replay через физически восстановленную SQL
с настоящей duplicate redelivery. Сохрани полный scope68+9+UI/physical/WAN/RU,
уже принятые решения и существующие C2/C4 согласования. Обновляй TODO/changelog,
коммить/push отдельными пакетами. Не проси повторять уже выданные разрешения.
