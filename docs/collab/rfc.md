# RFC: совместное редактирование DOCX в WordCraft

- Статус: Draft
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
 "payload":{"text":"Привет"}}
```

- `kind:"primitive"` — позиционная операция над моделью: `insert_text`, `delete_range`,
  `split_para`, `merge_para`, `set_run_props`, `set_para_props`, `insert_block`,
  `remove_block`, `move_block`, `insert_row`, `remove_row`, `insert_cell`, `set_style`,
  `add_comment`, `resolve_comment`, `set_field`.
- `kind:"command"` — нормализованная команда движка с явным адресом и контекстом
  (`pos`, `pending`, `author`); допустима для команд без побочных эффектов на сервере.
- `kind:"structural"` — эксклюзивно, `baseVersion == head`: вставка/удаление таблицы,
  `parts`, смена стиля/нумерации, paste фрагмента.
- `kind:"merge"` — комментарии и свойства, не зависящие от структуры (LWW по свойству).

Клиент выводит примитивы из своей локальной команды: `Arc::ptr_eq` по `body`/`parts`
даёт изменённые блоки бесплатно, внутри изменённого абзаца — минимальный текстовый дифф
по байтовым смещениям. Сырые `Session::run(id, params)` на проводе запрещены.

**Границы с движком.** Производное (layout, номера страниц, результаты полей, метки
списков) не пересылается — пересчитывается каждой репликой (`Session::layout()`).
`kept`/`passthrough` не пересылаются и живут на авторитетной копии (см. §4.6).

### 3.2. Идентификаторы (ADR-0002)

- Сервер при join строит карту `nodeId ↔ (story, path)` для абзацев/таблиц и выдаёт
  `partId`/`commentId`/`revisionId` из **серверных диапазонов** вместо локального
  `free_id()` (`doc/src/parts.rs:8`).
- Операции адресуют `(nodeId, off)`; индекс-путь — только внутри `nodeId`.
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
| `auth` | C→S | `token`, `clientId`, `lastVersion?` | первое сообщение; токен не в URL |
| `welcome` | S→C | `sessionId`, `role`, `participants`, `version`, `epoch`, `snapshot?` | вход в комнату |
| `op` | C→S | `opId`, `baseVersion`, `epoch?`, `kind`, `target`, `payload` | операция |
| `applied` | S→C | `opId`, `version`, `epoch`, `delta`\|`state` | авторитетный результат |
| `rejected` | S→C | `opId`, `code`, `retryable`, `details?` | отказ |
| `presence` | C↔S | `cursor`, `selection`, `viewport`, `active` | не журналируется |
| `snapshot` | S→C | `version`, `chunk`, `last` | догон/ресинк |
| `commit` | C→S | `opId`, `expectedVersion`, `etag?` | сборка файла |
| `committed` | S→C | `version`, `sha256`, `bytes?`, `warnings[]` | результат |
| `error` | S→C | `code`, `message`, `retryable` | вне операций |
| `ping`/`pong` | C↔S | `ts` | heartbeat |

Коды отказа: `stale` (retryable), `conflict`, `forbidden`, `gone`, `lease`,
`token-expired` (retryable), `too-large`, `invalid`.

### 4.3. Порядок и версии (ADR-0003)

- `version: u64` на документ, +1 на принятую операцию; `epoch` — на structural-операцию.
- `opId = (clientId, seq)`; сервер хранит окно ≥ 24 ч / ≥ 10 000 операций и на повтор
  возвращает сохранённый `applied`, **не исполняя** операцию заново (undo, relative
  moves и вставки неидемпотентны).
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

1. `auth{lastVersion}` → если версия в журнале: `welcome` + хвост `applied` по порядку.
2. Если версия вытеснена: `welcome` + `snapshot` (чанки) + `version`; клиент заменяет
   модель и сбрасывает непринятую очередь.
3. Непринятые операции клиент хранит с `opId` и переигрывает после `welcome`; повторный
   `opId` дедуплицируется сервером.

### 4.6. Снапшоты и compaction (ADR-0005)

- Снапшот сессии = **модель JSON** (`serde` `Document`) **плюс исходные байты `.docx`**,
  из которых она открыта. Причина: `Document::kept` помечен `#[serde(skip)]` и в JSON не
  попадает (`doc/src/lib.rs`), а без `kept` commit теряет немоделируемый XML правленых
  абзацев. Байты `.docx` дают `kept`/`passthrough` при восстановлении.
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
не влияет на `version`; viewer шлёт presence.

## 6. Undo/redo на пользователя (ADR-0010)

- Сервер ведёт per-user стек шагов: `{actorId, label, ops[]}`; шаг = операции одного
  пользовательского действия.
- Undo = **новая операция**: для `primitive`-текста — обратная вставка/удаление (точные
  `off`/`len` из журнала); для structural — компенсирующая операция из сохранённого
  before-образа узла; если обратной операции нет — `undo-conflict` без молчаливой
  перезаписи.
- Redo симметрично; чужие операции не откатываются; лимит шагов на пользователя (200) и
  TTL сессии.
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

- `commit{opId, expectedVersion, etag}` → сервер собирает `.docx` из авторитетной модели
  `io::save_bytes` (`engine/src/io.rs:441`) и отдаёт кабинету байты + `sha256` + `version`.
- `expectedVersion`/`etag` не совпал с текущим файлом → `409`; предложение «сохранить
  копию».
- Ответ несёт `warnings[]`: что именно не сохранится (правленые абзацы, чей `kept`
  выброшен; неизвестные части, если `passthrough` не было). Пустой `warnings[]` —
  round-trip без потерь по модели.
- Идемпотентность: повтор с тем же `opId`/версией возвращает тот же результат.
- Кабинет создаёт новую версию, предыдущая сохраняется (его ответственность).

## 9. Сервер

- `crates/collab` — протокол, состояние комнаты, журнал, дедуп, ID-карта, undo,
  транспортно-независимо (тестируется без сети).
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
- Интеграция: `CollabClient` получает операции из `Session::run` (дифф по `Arc::ptr_eq`),
  применяет чужие через отдельный путь (`apply_remote`), мимо undo и без `dirty`.
- Presence — из `view`/`sel`/`layout` (страница каретки); рендер чужих кареток — в canvas
  `ui-egui`.
- Viewer: команды, меняющие документ, отключены в UI, сервер отвечает `forbidden`.
- Track changes/комментарии: `author` = `sub` из JWT; `Session::author` выставляется сервером.

## 11. Тесты

| Тест | Уровень | Что проверяет |
|---|---|---|
| журнал/дедуп/порядок | unit (`crates/collab`) | `opId`-повтор не исполняется, `version` монотонен |
| сходимость текста | integration, 2–3 `Session` | двое печатают в одном абзаце → одинаковый `Document` |
| structural-конфликт | integration | вставка строки/абзаца параллельно → один `rejected`, состояния равны |
| reconnect | integration | обрыв, догон по `lastVersion`, повтор `opId` |
| роли | unit+integration | viewer получает `forbidden`, presence проходит |
| undo per-user | unit | отмена не откатывает чужое |
| большой документ | integration | 100+ страниц, 3 клиента, время apply |
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
