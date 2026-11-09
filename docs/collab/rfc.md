# RFC: совместное редактирование DOCX в WordCraft

- Статус: Draft (реализация — `STATUS.md`, PR #5834)
- Автор: `wordcraft/envoy`
- ADR: `../.craft/bus/decisions/0001…0010` (Phase 0 закрыт, 10/10 accepted)
- Upstream issue/PR: PR в `storytold/wordcraft`, ветка `feature/realtime-collaboration`
- Основание: `study.md`, `options.md` (этот каталог), `TASK.md`, `INTEGRATION.md`

## 1. Контекст

WordCraft — однопользовательский редактор: одна `Session`, один автор, undo — глобальные
снапшоты `Document` (`engine/src/session/history.rs:7`), сети в кодовой базе нет вовсе.
Модель — дерево историй (`body` + `parts`), абзацев (`String` + `Vec<Run{len, props}>`),
таблиц и inline-объектов; стабильных ID узлов нет, позиция — `Pos { story, path, off }`
(`doc/src/position.rs:15,60`). Мутация — `Session::run(id, &Value)`, 656 команд, команды
относительны выделения (`engine/src/session/run.rs:55`).

Phase 0 зафиксировал: единица операции — нормализованная intent-операция, исполняемая
сервером (ADR-0001), session-scoped ID (ADR-0002), серверный `version` + `opId`-дедуп
(ADR-0003), классы `merge`/`structural` (ADR-0004), снапшот + журнал (ADR-0005),
presence в общем WS (ADR-0006), один control plane + per-app адаптер (ADR-0007), JWT +
lease (ADR-0008), commit сервером (ADR-0009), per-user undo (ADR-0010).

## 2. Цели и не-цели

Цели (из `TASK.md`):
1. 2–5 участников правят один `.docx` в реальном времени; курсоры, выделения, участники.
2. Порядок, версии, идемпотентность, догон после reconnect.
3. Per-user undo/redo; чужие правки не откатываются.
4. Роли `owner/editor/commenter/viewer` проверяются сервером.
5. Commit сервером; `.docx` не ломается, немоделируемое сохраняется как раньше.
6. Фича за флагом; одиночный режим — по умолчанию.

Не-цели: аккаунты/облако/биллинг, несколько документов в одной сессии, офлайн-правка
(в v1 только догон), мобильный UX, полная совместимость с Word.

## 3. Модель данных для синхронизации

### 3.1. Единица операции (ADR-0001)

```json
{"opId":"c7:12","docId":"...","actorId":"sub","baseVersion":41,"epoch":7,
 "kind":"primitive|command|structural|merge",
 "target":{"story":"body","nodeId":18,"off":42},
 "payload":{"text":"Привет"},
 "history":{"action":"undo","opId":{"client":"c7","seq":11}}}
```

- `kind:"primitive"` — позиционная операция над моделью: `insert_text`, `delete_range`,
  `split_para`, `merge_para`, `set_run_props`, `set_para_props`, `insert_block`,
  `remove_block`, `move_block`, `insert_row`, `remove_row`, `insert_cell`, `set_style`,
  `add_comment`, `resolve_comment`, `set_field`.
- `kind:"command"` — нормализованная команда движка с явным адресом и контекстом:
  `payload: {"command":"format.bold","params":{...},"pending":<CharProps>}`; каретка
  берётся из `target`, `pending` — из payload, `author` — из JWT `sub` (сервер
  подставляет его в `Session`). Команда исполняется тем же `Session::run`, что и в UI.
  Допустимы только команды, меняющие документ, и только вне запрещённых областей
  (`file.`, `tools.`, `ui.`, `view.`, `caret.`, `select.`, `collab.`, `edit.undo`,
  `edit.redo`, `edit.repeat`, `review.readAloud`); остальные — `forbidden`.
- `kind:"structural"` — эксклюзивно, `baseVersion == head`: вставка/удаление таблицы,
  `parts`, смена стиля/нумерации, paste фрагмента.
- `kind:"merge"` — комментарии и свойства, не зависящие от структуры (LWW по свойству).

Клиент выводит примитивы из своей локальной команды: `Arc::ptr_eq` по `body`/`parts`
даёт изменённые блоки бесплатно, внутри изменённого абзаца — минимальный текстовый дифф
по байтовым смещениям. Сырые `Session::run(id, params)` на проводе запрещены.

`kind:"command"` — запасной путь для команд, чей эффект клиент не смог нормализовать в
примитивы. Авторитетная дельта такой операции повторяет саму операцию
(`{"op":"command","command","params","nodeId","off","pending"}`), чтобы остальные
реплики исполнили её локально при том же документе. Обратной операции для произвольной
команды нет: шаг per-user undo для `kind:"command"` не создаётся, а клиент получает
`label` и не показывает Undo для этого шага (ADR-0010, ограничение v1).

**Границы с движком.** Производное (layout, номера страниц, результаты полей, метки
списков) не пересылается — пересчитывается каждой репликой (`Session::layout()`).
`kept`/`passthrough` не пересылаются и живут на авторитетной копии (см. §4.6).

### 3.2. Идентификаторы (ADR-0002)

- Сервер при join строит карту `nodeId ↔ (story, path)` для абзацев, таблиц, их строк и
  ячеек (а также абзацев внутри ячеек) и выдаёт
  `partId`/`commentId`/`revisionId` из **серверных диапазонов** вместо локального
  `free_id()` (`doc/src/parts.rs:8`). Путь — это `doc::Path`: `[block]` — блок стори,
  `[block, row]` — строка таблицы, `[block, row, cell]` — ячейка,
  `[block, row, cell, block]` — абзац в ячейке.
- Операции адресуют `(nodeId, off)`; индекс-путь — только внутри `nodeId`.
- Структурная правка сдвигает индекс в **своём** контейнере
  (`insert_block`/`split_para` — блоки стори, `insert_row`/`remove_row` — строки
  таблицы): id ячейки переезжает вместе с содержимым, id соседних ячеек не меняется.
  Удалённый блок/строка уносит id поддерева в надгробия.
- Удалённые `nodeId` попадают в надгробия на окно журнала; операция по ним —
  `rejected{code:"gone", retryable:false}`.
- Ревизии track changes адресуются **индексом** в `Document::revisions`
  (`CharProps.ins/del`, `doc/src/props/char_props.rs:353`). При вставке/удалении ревизии
  сервер перенумеровывает ссылки и рассылает `primitive` `remap_revisions` — это часть
  операции, а не отдельный протокол.
- `media`-ключи — строки; сервер префиксует их `clientId`, чтобы два клиента не записали
  один ключ.

### 3.3. Документ на сервере

`Room = (docId)` хранит: `Document` (авторитетная модель), `version`, `epoch`,
журнал `Vec<JournalEntry>` с `opId`-окном, карту ID, надгробия, базовый `.docx` (для
`kept`/`passthrough`) и per-user стеки undo. Снапшот — §4.6.

## 4. Протокол

### 4.1. Транспорт

WebSocket, JSON text frames (UTF-8), один фрейм ≤ 1 MiB; TLS снаружи (reverse proxy).
`ping/pong` на уровне WS + heartbeat раз в 15 с. Отсутствие heartbeat 45 с → разрыв.
Бинарные фреймы зарезервированы под чанки снапшота.

### 4.2. Сообщения

| Тип | Направление | Поля | Семантика |
|---|---|---|---|
| `auth` | C→S | `token`, `client_id`, `last_version?` | первое сообщение; токен не в URL |
| `welcome` | S→C | `session_id`, `role`, `participants`, `version`, `epoch`, `snapshot?` | вход в комнату |
| `op` | C→S | `opId`, `baseVersion`, `epoch?`, `kind`, `target`, `payload` | операция |
| `applied` | S→C | `opId`, `version`, `epoch`, `delta`\|`state` | авторитетный результат |
| `rejected` | S→C | `opId`, `code`, `retryable`, `details?` | отказ |
| `presence` | C→S | `cursor`, `selection`, `viewport`, `active` | не журналируется |
| `presence` | S→C | `client_id`, `role`, `state` | `role` ставит сервер из JWT, не клиент |
| `snapshot` | S→C | `version`, `chunk`, `last` | догон/ресинк |
| `commit` | C→S | `op_id`, `expected_version`, `etag?` | сборка файла |
| `committed` | S→C | `version`, `sha256`, `bytes?`, `warnings[]` | результат |
| `error` | S→C | `code`, `message`, `retryable` | вне операций |
| `ping`/`pong` | C↔S | `ts` | heartbeat |

Коды отказа: `stale` (retryable), `conflict`, `forbidden`, `gone`, `lease`,
`token-expired` (retryable), `too-large`, `invalid`.

### 4.3. Порядок и версии (ADR-0003)

- `version: u64` на документ, +1 на принятую операцию; `epoch` — на structural-операцию.
- `opId = (clientId, seq)`; сервер хранит окно ≥ 24 ч / ≥ 10 000 операций и на повтор
  возвращает сохранённый `applied`, **не исполняя** операцию заново (undo, relative
  moves и вставки неидемпотентны). Окно переживает восстановление из снапшота (§4.6).
- `baseVersion` — контекст, не блокировка; для `structural` — строго `baseVersion == head`.
- Presence не увеличивает `version`.

### 4.4. Конфликты (ADR-0004)

| Ситуация | Политика | Что видит пользователь |
|---|---|---|
| Двое печатают в одном абзаце | серверный порядок, `insert_text` со сдвигом `off` | текст обоих, каретка не прыгает |
| Правка в удалённом абзаце | `rejected{code:"gone"}` | «абзац удалён», курсор на соседний |
| Structural против structural | очередь, второй ждёт `head` | индикатор «синхронизация…» |
| Смена стиля/нумерации | `structural`, эксклюзивно | краткая блокировка, затем пересчёт |
| Комментарий к удалённому якорю | `rejected` или перенос к ближайшему узлу по ADR-0004 | бейдж «комментарий потерял якорь» |
| Устаревшая база | `applied` с `conflict:true` + дельта | слияние без диалога |

OT/CRDT не используются: сервер применяет операцию к head и возвращает результат.

### 4.5. Reconnect и догон

1. `auth{last_version}` → если версия в журнале: `welcome` + хвост `applied` по порядку.
2. Если версия вытеснена: `welcome` + `snapshot` (чанки) + `version`; клиент заменяет
   модель и сбрасывает непринятую очередь.
3. Непринятые операции клиент хранит с `opId` и переигрывает после `welcome`; повторный
   `opId` дедуплицируется сервером.

### 4.6. Снапшоты и compaction (ADR-0005)

- Снапшот сессии = **модель JSON** (`serde` `Document`) **плюс исходные байты `.docx`**,
  из которых она открыта. Причина: `Document::kept` помечен `#[serde(skip)]` и в JSON не
  попадает (`doc/src/lib.rs`), а без `kept` commit теряет немоделируемый XML правленых
  абзацев. Байты `.docx` дают `kept`/`passthrough` при восстановлении.
- Вместе с ними хранится **окно `opId`** — сохранённые `applied`-ответы до версии
  снапшота (`dedup.json`, oldest first). Это состояние сервера, на провод оно не
  выходит: повтор после рестарта — обычная `op` с тем же `opId`, ответ — сохранённый
  `applied`, `version` не двигается. Окно обрезается до версии снапшота и до лимита
  10 000, вытесняя старейшие; снапшот без `dedup.json` открывается с пустым окном.
- Хранится 2 снапшота; журнал обрезается по версии снапшота; восстановление = снапшот +
  реплей хвоста.
- Частота: 500 операций / 5 минут / перед подключением нового участника при длинном
  журнале. Commit — отдельная операция и отдельный формат (`.docx`), падение writer'а не
  ломает сессию.

> **Расхождение с ADR-0005.** ADR-0005 (строка про wordcraft) говорит «модель/`kept`/
> `passthrough`». Это неверно для `kept` (`#[serde(skip)]`). Реализация следует факту;
> прошу chair выпустить уточняющий ADR (supersedes) до merge.

## 5. Presence (ADR-0006)

Эфемерный тип в том же WS: `{cursor:{story,nodeId,off}, selection?, viewport:{page,scroll},
active:{tab,pane}}`. 5–10 Гц, ≤ 2 KiB, коалесцирование в последнее состояние,
`drop-stale` при перегрузке, TTL 30 с, `leave`/разрыв чистит сразу. Не журналируется,
не влияет на `version`; автору своё состояние не эхо-броадкастится. viewer шлёт presence.
Сервер добавляет в броадкаст `role` из JWT (ADR-0008), чтобы пир, подключённый
раньше, видел роль нового участника с первого же кадра.

## 6. Undo/redo на пользователя (ADR-0010)

- Сервер ведёт per-user стек шагов: `{actorId, label, version, inverse[], forward[]}`;
  шаг = операции одного пользовательского действия.
- Запрос — **новая операция** с полем `history`: `{"action":"undo","opId":<id шага>}`
  или `{"action":"redo",...}`. Отдельного сообщения `undo` в конверте нет: так undo
  проходит те же проверки роли, `opId`-дедупа и `baseVersion`, что и правка, и получает
  ровно один `version`. Обратные операции шага исполняются под этой операцией и
  отдельно не журналируются.
- `opId` в `history` — id операции, создавшей шаг. Сервер берёт только **новейший**
  шаг актора; старый id (устаревшее меню) → `undo-conflict`.
- Конфликт: если после шага другая операция трогала тот же `nodeId` (primitive) или
  документ пережил structural-операцию (structural-шаг) — `undo-conflict`, ничего не
  меняется. Молчаливой перезаписи нет.
- Шаг из нескольких операций исполняется транзакцией: все обратные операции
  применяются под одной версией, и если одна отказывает, уже применённые
  откатываются — документ не остаётся между состояниями. Реплики получают шаг
  одной дельтой `{"op":"transaction","ops":[…]}` и применяют её на копии
  документа и карты узлов: либо всё, либо ничего.
- `kind:"command"` не имеет обратной операции, поэтому он — **барьер**: шаг
  записывается без `inverse`, undo на нём отвечает `unsupported` и не перескакивает к
  более старому шагу.
- Redo симметрично; после undo шаг сохраняет версию undo, поэтому redo разрешён, пока
  никто не тронул цель снова. Чужие операции не откатываются; лимит 200 шагов на
  пользователя, TTL сессии.
- Локальный undo-стек `Session` не участвует: применение чужих операций и undo идут
  отдельным путём, мимо `checkpoint()` (`engine/src/session/history.rs:7`).

## 7. Роли и права (ADR-0008)

JWT первым сообщением: claims `{sub, aud, doc, app, role, sid, iat, exp, ver}`,
асимметричная подпись (ES256). Роль проверяется на каждой операции:

| Роль | Presence | Snapshot | Op | Commit |
|---|---|---|---|---|
| owner | да | да | да | да |
| editor | да | да | да | нет |
| commenter | да | да | только `add_comment`/`resolve_comment` | нет |
| viewer | да | да | `forbidden` | нет |

`author` операции подставляет сервер из `sub` — клиентский `author` не принимается.
Lease — одна комната на файл, TTL + heartbeat; отзыв закрывает WS по `sid`.

## 8. Commit и версии (ADR-0009)

- `commit{op_id, expected_version, etag}` → сервер собирает `.docx` из авторитетной модели
  `io::save_bytes` (`engine/src/io.rs:441`) и отдаёт кабинету байты + `sha256` + `version`.
- `expected_version`/`etag` не совпал с текущим файлом → `409`; предложение «сохранить
  копию».
- Ответ несёт `warnings[]`: что именно не сохранится (правленые абзацы, чей `kept`
  выброшен; неизвестные части, если `passthrough` не было). Пустой `warnings[]` —
  round-trip без потерь по модели.
- Идемпотентность: повтор с тем же `opId`/версией возвращает тот же результат.
- Кабинет создаёт новую версию, предыдущая сохраняется (его ответственность).

## 9. Сервер

- `crates/collab` — протокол, состояние комнаты, журнал, дедуп, ID-карта, undo,
  транспортно-независимо (тестируется без сети).
- `crates/collab-doc` — адаптер модели `Document` к позиционным примитивам.
- `crates/collab-engine` — модель комнаты поверх `Session`: команды движка идут тем же
  `Session::run`, что и в UI, с автором из JWT и без серверной истории undo.
- `crates/collab-client` — состояние клиента (opId, очередь, reconnect) без транспорта;
  `crates/collab-ws` — WebSocket-транспорт, который ведёт это состояние.
- `crates/collab-app` — сессия приложения: карта узлов, применение дельт, план локальной
  команды (дифф → примитивы), presence и участники; транспорт сокета остаётся в `collab-ws`.
- `apps/wordcraft-collab-server` — бинарь: `tokio` + `axum` WS, JWT, комнаты, снапшоты,
  commit; конфиг из env (`COLLAB_BIND`, `COLLAB_JWT_PUB`, `COLLAB_DATA_DIR`).
- Healthcheck `GET /healthz`; метрики `GET /metrics` (Prometheus-текст): комнаты,
  операции/с, отказы, задержка apply, размер журнала.
- Лимиты: документ ≤ 64 MiB, фрейм ≤ 1 MiB, участников ≤ 8, 200 операций/с на комнату,
  журнал ≤ 100 000 операций (дальше — принудительный снапшот).
- Логи без содержимого документа и токенов; в журнале — `sub` и `opId`.

## 10. Клиент

- **Feature flag:** `collab` в `Prefs` (default off) + env `WORDCRAFT_COLLAB=1` для
  headless; в web — `?collab=1`. Без флага ни одного сетевого вызова.
- Токен — из окружения (`WORDCRAFT_COLLAB_TOKEN`) или host-сообщения кабинета; в
  `Prefs` и URL он не попадает.
- Интеграция: перед командой `WordApp` запоминает `Document` (клонирование делит
  блоки через `Arc`), после — `collab-doc::diff` даёт примитивы (`replace_range`,
  `split_para`, `merge_para`, `insert_block`, `remove_block`, `insert_row`,
  `remove_row`). Дифф заходит в ячейки таблиц: печать в ячейке — `replace_range` по
  id абзаца ячейки, вставка/удаление строки — `insert_row`/`remove_row` с телом
  строки (обратная операция берёт его из дельты). Форматирование, вставка/удаление
  столбца, merge/split ячеек и несколько перемещений дифф не выражает — уходит
  `kind:"command"` с явным `selection`; такой шаг сервер исполняет тем же
  `Session::run`. Команда, меняющая форму таблицы, снимает id затронутых таблиц
  (надгробия) и выдаёт их заново: устаревший id ячейки получает `gone`, а не правит
  чужую ячейку.
- Чужие дельты применяются `collab-engine::apply_delta`: примитивы — напрямую в
  `Document` с пересчётом layout, команды — `Session::run_remote`, который не пишет шаг
  в локальный undo. Свои `applied` не применяются повторно (UI уже показал правку).
- Presence — из `sel`/`view`; 5–10 Гц, панель участников и чужие каретки рисуются в
  `ui-egui` (`collab.rs`).
- Viewer: команды, меняющие документ, отключены в UI до вызова движка, сервер
  дополнительно отвечает `forbidden`.
- `edit.undo`/`edit.redo` в совместной сессии уходят в комнату, а не в локальный
  snapshot-undo; одиночный режим не меняется.
- Track changes/комментарии: `author` = `sub` из JWT; `Session::author` выставляется
  сервером.

## 11. Тесты

| Тест | Уровень | Что проверяет |
|---|---|---|
| журнал/дедуп/порядок | unit (`crates/collab`) | `opId`-повтор не исполняется, `version` монотонен |
| сходимость текста | integration, 2–3 `Session` | двое печатают в одном абзаце → одинаковый `Document` |
| дельта → реплика | integration (`collab-doc`) | `apply_delta` после каждой операции даёт тот же JSON документа |
| диff → операция | integration (`collab-doc`) | `Enter`/`Backspace`/вставка абзаца превращаются в `split_para`/`merge_para`/`insert_block` |
| structural-конфликт | integration | вставка строки/абзаца параллельно → один `rejected`, состояния равны |
| reconnect | integration (socket) | обрыв до ответа, повтор `opId`, догон по `lastVersion` |
| presence | integration (socket) | состояние доходит до второго клиента, `version` не растёт |
| роли | unit+integration | viewer получает `forbidden`, presence проходит |
| undo per-user | unit+integration | одна версия на undo, `undo-conflict` при чужой правке, команда — барьер |
| большой документ | integration (`collab-engine`) | 3000 абзацев, 100 правок, id на каждый абзац |
| commit | integration | файл открывается LibreOffice без repair; `warnings[]` |
| track changes | integration | автор из токена, ревизии не разъезжаются |

Headless: `cargo test -p wordcraft-collab`, `wordcraft-cli run --script` для сценариев.

## 12. Безопасность

JWT только первым сообщением, не в URL; ключ из env, не в репозитории. Origin/`aud`
проверяются. Лимиты на размер фрейма, число комнат и участников. Снапшоты и журнал — на
диске с правами владельца процесса. В логи не попадают bytes документа, токены,
`payload` операций (только `opId`, `kind`, `version`, `sub`).

## 13. Открытые вопросы

1. ADR-0005: уточнить формулировку про `kept` (см. §4.6).
2. Подпись JWT: ES256 (по ADR-0008) — нужен ключ кабинета; до интеграции — тестовый ключ.
3. `commenter` в WordCraft: движок не различает «только комментарии»; сервер режет по
   allowlist `kind/target`, UI блокирует остальное.
4. Кто инициирует commit при выходе последнего участника, если он viewer (ADR-0009, п. 3).
5. Хранение: файловая система + `journal.jsonl` на комнату (v1) или Postgres (v2).

## 14. План поставки

1. **PR-1:** `crates/collab` — конверт, журнал, дедуп, ID-карта, undo-стек, тесты без сети.
2. **PR-2:** `apps/wordcraft-collab-server` — WS, JWT, presence, снапшоты, healthz/metrics,
   интеграционные тесты 2–3 клиента.
3. **PR-3:** клиент — `CollabClient`, feature flag, presence-рендер, блокировка viewer.
4. **PR-4:** commit + host-bridge (`wordcraft:session`, `presence`, `commit`, `committed`),
   round-trip gate (LibreOffice), документация.

PR-1 не меняет поведение приложения: только новый crate и тесты. CI не трогаем (house rules).
