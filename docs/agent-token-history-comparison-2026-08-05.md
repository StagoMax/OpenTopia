# OpenTopia 与 Codex 同提示词会话：完整 Token 历史及验收机制审计

生成日期：2026-08-05（Asia/Shanghai）

## 1. 范围与数据源

本报告只比较以下两次首轮任务，不包含 Codex 后续的 `rgb(243,243,244)`、撤销等 follow-up：

- OpenTopia xhigh turn：`7d709887-2657-40af-8b9d-98f8b1e3e35e`
- OpenTopia thread：`ffd15beb-1ea1-431b-b313-b1c8c954aff2`
- OpenTopia 原始数据库：`J:\Project\OpenTopia\.opentopia\opentopia.db`
- Codex 首轮 turn：`019fd0d9-de1d-74e1-8603-abe4c269aac9`
- Codex 原始会话：`C:\Users\Stargo\.codex\sessions\2026\08\05\rollout-2026-08-05T15-36-05-019fd0d9-d67f-7c51-a090-330e674c1a93.jsonl`

重要口径：

- `reasoning` 是 `output` 的子集，不能在 `total` 之外再加一次。两套日志中的 `total` 都是 `input + output`。
- `cached input` 是 `input` 的子集。命中缓存不表示这些内容没有出现在请求上下文中。
- 日志保存的是 Provider 返回的聚合 token 数，不保存逐 token 的字符串、token ID 或 tokenizer 切分结果。因此本文所说“完整 Token 历史”是日志中可获得的每轮完整 usage 字段，而不是不存在于日志中的 token ID 序列。
- OpenTopia 请求体大小是把数据库里的 `body` 按紧凑 JSON、UTF-8 重新序列化得到的字节数；它不包含 HTTP header，也不是网络抓包长度。
- 本报告不展示隐藏推理正文，只使用请求元数据、usage、公开消息、工具事件和计划状态。

## 2. 结论

1. OpenTopia 这次不是在第一轮就上传了 25 万 token；它从第 1 轮的 `13,547 input tokens` 增长到第 32 轮的 `252,746 input tokens`。第 33 轮请求体已达到 `861,459` 字节，随后等待 `90.569` 秒返回 504，但 504 响应没有 usage，所以无法从客户端日志得知第 33 轮被服务端实际 tokenizer 计数了多少。
2. “上传 token 太多”是 504 的强相关因素，但现有日志不能单独证明因果。日志能证明的是：最后一次请求很大、使用 xhigh、网关约 90 秒后超时；缺少上游网关和模型服务端的排队、首包及推理耗时日志，不能排除上游拥塞或服务故障。
3. OpenTopia 在失败前有 33 个模型轮次、34 次 Provider 尝试；Codex 首轮有 14 条 `token_count`。按所有 Provider 尝试合计，OpenTopia 处理了 `4,496,981 input tokens`，约为 Codex `538,045` 的 `8.36` 倍。
4. OpenTopia 原架构**已经有明确的结构化验收清单**：每个计划步骤都带 `acceptanceCriteria` 和 `evidence`，还有 finalization guard。这次 xhigh 会话也实际创建并使用了这套清单。问题不是“完全没有验收架构”。
5. OpenTopia 的 guard 强制的是可观察状态：不能有 pending/in-progress 步骤，completed 步骤必须有非空 evidence。它不会自行验证“evidence 是否真的足以证明每一条 acceptance criterion”；语义正确性仍依赖模型。
6. Codex 该首轮日志里没有结构化计划或显式 acceptance-criteria 对象，也没有 `update_plan` 调用。它使用了两次公开范围确认、补丁、最终 diff 和两项检查，属于**非结构化但实际执行了的验收过程**。

## 3. OpenTopia xhigh：完整逐轮 Token 记录

OpenTopia turn 从 `2026-08-05T08:35:28.969806600Z` 运行到 `2026-08-05T08:48:20.738733Z`，状态为 `failed`，总时长约 12 分 51.8 秒。

表中第 11 轮有两次请求：第一次流式结果中的 `update_plan` 参数违反 Provider contract，随后用非流式 transport 重试。第 11 轮尝试 1 的 usage 来自原始 `token_usage` 事件（event seq 713）；尝试 2 的 usage 来自 `provider_response_received.body.usage`。第 33 轮为 504，Provider 没有返回 usage。`未返回`严格保留原始日志语义，不擅自当作 0。

|轮|尝试|发送时间 UTC|上下文估算|请求体字节|消息数|状态|等待秒|input|cached input|output|reasoning|total|
|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
|1|1|08:35:37.616|5,726|52,401|15|200|53.617|13,547|未返回|252|158|13,799|
|2|1|08:36:31.367|111,863|53,673|18|200|34.243|14,581|未返回|559|471|15,140|
|3|1|08:37:05.826|111,954|54,168|20|200|43.525|14,677|13,056|616|345|15,293|
|4|1|08:37:49.438|112,332|55,883|22|200|13.329|14,987|未返回|480|175|15,467|
|5|1|08:38:02.866|113,552|61,154|24|200|10.649|16,028|未返回|325|150|16,353|
|6|1|08:38:13.783|141,806|190,174|26|200|9.267|55,769|15,104|312|200|56,081|
|7|1|08:38:23.298|142,337|192,604|28|200|12.220|56,369|54,016|426|261|56,795|
|8|1|08:38:35.688|174,146|333,065|30|200|11.847|96,827|55,040|406|302|97,233|
|9|1|08:38:47.914|175,992|341,131|32|200|21.137|98,831|94,976|669|484|99,500|
|10|1|08:39:09.482|180,345|360,918|34|200|22.764|104,412|97,024|621|516|105,033|
|11|1|08:39:32.613|181,169|364,596|36|200|34.505|105,295|103,168|517|370|105,812|
|11|2|08:40:07.120|181,169|364,557|36|200|0.048|105,295|104,192|760|516|106,055|
|12|1|08:40:07.361|182,479|370,205|38|200|7.897|106,472|104,192|264|112|106,736|
|13|1|08:40:15.464|197,167|437,797|40|200|15.520|127,680|105,216|680|502|128,360|
|14|1|08:40:31.165|198,390|443,087|42|200|9.311|128,777|126,720|336|244|129,113|
|15|1|08:40:40.851|198,872|445,299|44|200|10.244|129,296|127,744|354|194|129,650|
|16|1|08:40:51.290|212,696|508,887|46|200|17.347|148,887|127,744|471|321|149,358|
|17|1|08:41:09.464|227,588|570,293|48|200|19.623|166,426|147,200|682|516|167,108|
|18|1|08:41:29.944|228,645|574,487|50|200|18.422|167,496|164,608|697|592|168,193|
|19|1|08:41:48.822|228,878|575,637|52|200|10.754|167,751|165,632|302|195|168,053|
|20|1|08:42:00.015|229,121|576,845|54|200|17.734|168,011|166,656|723|606|168,734|
|21|1|08:42:18.186|229,341|577,942|56|200|14.394|168,251|166,656|590|510|168,841|
|22|1|08:42:32.964|229,436|578,454|58|200|14.909|168,350|166,656|629|482|168,979|
|23|1|08:42:48.795|237,513|612,855|60|200|24.509|176,728|166,656|663|527|177,391|
|24|1|08:43:14.286|242,156|633,414|62|200|22.361|183,254|7,936|813|700|184,067|
|25|1|08:43:37.091|242,360|634,390|64|200|12.578|183,476|182,016|479|285|183,955|
|26|1|08:43:50.487|243,569|639,232|66|200|16.506|184,589|182,016|699|516|185,288|
|27|1|08:44:07.232|268,598|755,082|68|200|21.260|221,925|183,040|912|786|222,837|
|28|1|08:44:29.478|273,367|776,123|70|200|15.142|228,787|220,928|574|465|229,361|
|29|1|08:44:45.128|281,083|809,197|72|200|35.647|238,847|227,072|693|571|239,540|
|30|1|08:45:21.772|282,760|816,636|74|200|22.519|240,733|237,312|868|737|241,601|
|31|1|08:45:45.185|283,775|821,024|76|200|22.268|241,881|239,360|984|874|242,865|
|32|1|08:46:08.462|292,621|859,633|78|200|41.091|252,746|240,384|1,739|1,639|254,485|
|33|1|08:46:50.113|293,018|861,459|80|504|90.569|未返回|未返回|未返回|未返回|未返回|

### 3.1 OpenTopia 的三种累计口径

重试使数据库里出现三种都值得保留的口径：

|口径|input|cached input|output|reasoning|total|说明|
|---|---:|---:|---:|---:|---:|---|
|32 条 `token_usage` 事件|4,391,686|3,888,128|19,335|14,806|4,411,021|运行时收到的流式 usage；记录了第 11 轮尝试 1，没有记录非流式尝试 2|
|每轮最终被接受的 200 响应|4,391,686|3,889,152|19,578|14,952|4,411,264|第 11 轮采用尝试 2，其他轮采用唯一成功响应|
|所有实际 Provider 尝试|4,496,981|3,992,320|20,095|15,322|4,517,076|把第 11 轮两次尝试都计入；最接近上游实际处理量，但不保证等同账单|

34 次请求体按上述紧凑 JSON 口径累计为 `16,302,302` UTF-8 字节。第 33 轮没有 usage，因此不在 token 累计数内。

值得注意的是，第 24 轮 Provider 只报告 `7,936 cached input`，下一轮又恢复到 `182,016`。这是 Provider 原始返回值，表中没有修正或平滑处理。

## 4. Codex 首轮：完整逐轮 Token 记录

Codex `task_complete` 记录总时长 `201,999 ms`，time-to-first-token 为 `118,521 ms`。表中数据逐条来自首轮范围内的 `event_msg.payload.type = token_count`；`本轮`来自 `last_token_usage`，`累计`来自 `total_token_usage`。由于这是该 session 的第一轮任务，最终累计就是本次任务累计。

Codex JSONL 不记录每轮完整的 on-wire HTTP 请求体和其字节数，因此不能像 OpenTopia 一样进行请求体字节对比。

|轮|JSONL 行|时间 UTC|本轮 input|本轮 cached|本轮 cache write|本轮 output|本轮 reasoning|本轮 total|累计 input|累计 cached|累计 output|累计 reasoning|累计 total|
|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
|1|16|07:38:06.058|18,393|6,912|0|239|104|18,632|18,393|6,912|239|104|18,632|
|2|20|07:38:10.434|19,030|17,152|0|128|8|19,158|37,423|24,064|367|112|37,790|
|3|24|07:38:14.994|26,761|18,176|0|136|43|26,897|64,184|42,240|503|155|64,687|
|4|28|07:38:19.567|37,011|26,368|0|155|59|37,166|101,195|68,608|658|214|101,853|
|5|32|07:38:24.406|37,881|36,608|0|170|19|38,051|139,076|105,216|828|233|139,904|
|6|36|07:38:29.353|38,138|37,632|0|179|22|38,317|177,214|142,848|1,007|255|178,221|
|7|43|07:38:36.252|40,107|37,632|0|382|72|40,489|217,321|180,480|1,389|327|218,710|
|8|48|07:38:41.136|42,397|39,680|0|182|104|42,579|259,718|220,160|1,571|431|261,289|
|9|52|07:38:46.719|43,147|41,728|0|258|101|43,405|302,865|261,888|1,829|532|304,694|
|10|61|07:39:00.499|45,646|42,752|0|909|506|46,555|348,511|304,640|2,738|1,038|351,249|
|11|66|07:39:06.649|46,579|44,800|0|241|140|46,820|395,090|349,440|2,979|1,178|398,069|
|12|72|07:39:20.867|46,969|45,824|0|192|40|47,161|442,059|395,264|3,171|1,218|445,230|
|13|76|07:39:24.279|47,299|45,824|0|90|32|47,389|489,358|441,088|3,261|1,250|492,619|
|14|80|07:39:28.911|48,687|46,848|0|124|46|48,811|538,045|487,936|3,385|1,296|541,430|

## 5. 两套 Token 历史对比

|指标|OpenTopia xhigh|Codex 首轮|比值/说明|
|---|---:|---:|---|
|模型轮次|33|14|OpenTopia 2.36 倍；OpenTopia 第 33 轮失败|
|Provider 尝试|34|日志中 14 条 usage|OpenTopia 第 11 轮有一次额外重试|
|累计 input|4,496,981|538,045|按 OpenTopia 所有 Provider 尝试，为 8.36 倍|
|累计 cached input|3,992,320|487,936|8.18 倍|
|派生未缓存 input|504,661|50,109|`input - cached`，OpenTopia 为 10.07 倍|
|累计 output|20,095|3,385|5.94 倍|
|累计 reasoning|15,322|1,296|11.82 倍|
|累计 total|4,517,076|541,430|8.34 倍|
|最后一个有 usage 的 input|252,746|48,687|OpenTopia 第 32 轮为 Codex 最大一轮的 5.19 倍|
|最终请求体|861,459 字节|未记录|OpenTopia 第 33 轮，随后 504|
|任务结果|12 分 51.8 秒后 504|201.999 秒成功|Codex 首次输出前包含 118.521 秒等待|

这里的 OpenTopia 累计采用“所有实际 Provider 尝试”，以免隐藏第 11 轮重试成本。若只比较每轮最终接受的响应，OpenTopia total 为 `4,411,264`，仍为 Codex 的约 `8.15` 倍。

### 5.1 能否认定 504 就是“上传 token 太多”

可观察证据支持“请求规模是重要风险因素”：

- 第 32 轮已成功处理 `252,746 input tokens`，其中 `240,384` 是 cached input。
- 第 33 轮上下文估算为 `293,018`，请求 JSON 为 `861,459` 字节、80 条 messages，并继续携带 39 个工具定义。
- 第 33 轮等待 `90.569` 秒后由上游返回 504。

但不能从这些客户端日志推出唯一因果：

- 504 响应没有 usage，因此没有第 33 轮的 Provider 精确 token 计数。
- 日志没有上游网关的排队时间、连接时间、首包时间、模型推理时间和 request-id 对应的服务端 trace。
- 504 也可能由上游拥塞、网关固定超时或模型服务异常触发。

因此严谨结论是：**大请求和 xhigh 很可能显著增加了超时概率，但要确认根因，还需要 nowcoding.ai 网关/上游模型服务端日志。**

## 6. 在哪里查看完整对话内容与 Token 日志

### 6.1 OpenTopia

最完整的数据源不是桌面启动日志，而是：

```text
J:\Project\OpenTopia\.opentopia\opentopia.db
```

可用 DB Browser for SQLite 打开 `events` 表，以 `turn_id` 过滤。若复制数据库到其他位置分析，应用运行期间还应一起复制 `opentopia.db-wal` 和 `opentopia.db-shm`，否则可能漏掉尚未 checkpoint 的最新事件。

关键事件：

- `model_context_built`：上下文估算和组成项。
- `model_request`：OpenTopia 内部模型请求快照。
- `provider_request_sent` / `provider_request_retried`：实际适配器请求 body，包含完整 messages 和 tools。
- `provider_response_received`：HTTP 状态及 Provider usage。
- `token_usage`：流式 usage 事件。
- `tool_call_started` / `tool_call_finished`：工具参数和结果。
- `plan_updated`：结构化计划、acceptanceCriteria 和 evidence。

查看每次 Provider 返回的原始 usage：

```sql
SELECT
  seq,
  created_at,
  json_extract(payload_json, '$.round') AS round,
  json_extract(payload_json, '$.attempt') AS attempt,
  json_extract(payload_json, '$.status') AS status,
  json_extract(payload_json, '$.body.usage') AS usage
FROM events
WHERE turn_id = '7d709887-2657-40af-8b9d-98f8b1e3e35e'
  AND kind = 'provider_response_received'
ORDER BY seq;
```

查看完整 outbound messages/tools（内容可能包含源码、附件和敏感信息，不要直接公开分享）：

```sql
SELECT
  seq,
  created_at,
  kind,
  json_extract(payload_json, '$.round') AS round,
  json_extract(payload_json, '$.attempt') AS attempt,
  json_extract(payload_json, '$.body') AS complete_provider_body
FROM events
WHERE turn_id = '7d709887-2657-40af-8b9d-98f8b1e3e35e'
  AND kind IN ('provider_request_sent', 'provider_request_retried')
ORDER BY seq;
```

桌面渲染日志位于：

```text
C:\Users\Stargo\AppData\Roaming\OpenTopia Dev\logs\startup-2026-08-05T07-34-15-144Z-36340.jsonl
```

这个文件主要记录 UI 收到/绘制事件，不是完整 Provider token 数据源；本次 turn 从该文件第 75 行附近开始出现。

### 6.2 Codex

原始会话文件：

```text
C:\Users\Stargo\.codex\sessions\2026\08\05\rollout-2026-08-05T15-36-05-019fd0d9-d67f-7c51-a090-330e674c1a93.jsonl
```

首轮边界：

- 第 2 行：`task_started`，turn id 为 `019fd0d9-de1d-74e1-8603-abe4c269aac9`。
- 第 80 行：本轮最后一个 `token_count`。
- 第 81 行：`task_complete`。
- 第 83 行开始是后续 `rgb(243,243,244)` turn，不属于本报告。

其中：

- `event_msg.payload.type = token_count` 保存 `last_token_usage` 和 `total_token_usage`。
- `response_item` 保存可观察的消息、工具调用和工具输出。
- 该 JSONL 没有保存逐 token ID，也没有保存每轮完整 on-wire HTTP request body。

如果需要 token ID 级别的拆分，必须另外取得该模型实际使用的 tokenizer 及精确请求文本后重新编码；即使重编码，也只能在 tokenizer 版本完全一致时视为准确。当前两份日志本身无法还原 Provider 内部最终使用的 token ID 序列。

## 7. OpenTopia 验收清单与 finalization guard 审计

### 7.1 架构层：确实存在

OpenTopia 的 `TaskPlanStep` 原生包含：

- `acceptance_criteria`
- `evidence`

定义见 `crates/opentopia-core/src/model.rs:516-529`；计划还会把每一条 Acceptance 和 Evidence 渲染回模型上下文，见 `model.rs:600-627`。

`set_plan` 的 schema 要求每个步骤至少一条 acceptance criterion：

- `crates/opentopia-core/src/tools.rs:1438-1446`
- `crates/opentopia-core/src/tools.rs:1503-1523`

将步骤标记 completed 时，工具要求 acceptance criteria 和 evidence 都非空：

- `crates/opentopia-core/src/tools.rs:1753-1767`

`complete_task` 自身要求 summary、verification 和 remaining_work，见：

- `crates/opentopia-core/src/tools.rs:1283-1307`
- `crates/opentopia-core/src/tools.rs:1341-1385`

finalization guard 会阻止以下状态直接结束：

- pending tool calls
- pending approvals
- 已存在计划中的 in-progress 步骤
- 已存在计划中的 pending 步骤
- 未结束的子 agent 或未读 mailbox

实现见 `crates/opentopia-core/src/agent.rs:1159-1331`。其中 pending/in-progress 计划阻断位于 `agent.rs:1207-1243`。

在 Goal mode 下，`complete_task` 还有额外硬校验：计划不得有 actionable steps，且 completed 步骤必须有 evidence，见 `crates/opentopia-core/src/tools.rs:1314-1336`。计划和验收字段也有持久化表结构，见 `crates/opentopia-core/src/store.rs:662-689`。

### 7.2 这次 xhigh 会话：实际使用了

本次 OpenTopia turn 的 `plan_updated` event seq 201 创建了三步结构化清单：

1. `inspect`
   - 阅读桌面设计规范和 token 定义。
   - 找到红框对应组件和选择器。
   - 理解相关文件已有 worktree 变更。
2. `edit`
   - 只更新相关 UI 样式。
   - 两个红框都保留背景并移除边框。
   - 不改无关视觉样式。
3. `verify`
   - 运行 design check。
   - 运行桌面 type check 或最强可用替代。
   - 检查最终 diff 的范围与正确性。

event seq 719 把 `inspect` 标为 completed 并附上三条 evidence；event seq 736 把 `edit` 标为 in-progress。随后任务在 edit 阶段遭遇 504，未执行修改，turn change set 是 empty。

因此，对“原来的架构没有验收清单吗”的答案是：**有，而且本次确实创建了；它没有机会走到最终验收，因为会话在编辑阶段就因 504 中止。**

### 7.3 强制边界：结构状态强制，语义真实性不强制

当前 guard 能证明：

- 模型至少写下了验收条件。
- 模型不能在计划仍 pending/in-progress 时正常 finalization。
- 模型把步骤标记 completed 时必须附上非空 evidence。

当前 guard 不能独立证明：

- evidence 与工具原始输出一致。
- evidence 足以覆盖每一条 acceptance criterion。
- 截图中的目标识别一定正确。
- CSS 最终级联效果与模型描述一致。

这是“有验收机制”与“有独立语义验证器”的区别。现有机制属于前者。

## 8. Codex 该会话是否有验收清单

在该 Codex 首轮 JSONL 中没有观察到：

- 结构化 plan 对象。
- `update_plan` / `set_plan` 工具调用。
- acceptance criteria 字段。
- 类似 OpenTopia `runtime_finalization_guard` 的可观察事件。

但它有实际的非结构化验收过程：

- 第 12 行公开说明要处理截图中的两处目标并做桌面检查。
- 第 56 行再次确认两类目标以及“仅移除描边，背景、圆角、间距与文字样式保持不变”。
- 第 58/59 行对应补丁执行及成功结果。
- 第 64 和 70 行对应 design check 与 desktop typecheck；第一次被 PowerShell policy 拦截后改用 `pnpm.cmd`。
- 第 74 行执行 `git diff --check` 和最终 `git diff`。
- 第 78 行 final answer 对两个目标和两项检查逐项交代。

所以答案是：**Codex 该会话没有显式、持久化、机器可检查的验收清单；它依靠模型在公开消息中形成范围约束，并实际执行 diff 与测试来完成验收。** OpenTopia 的架构在“验收条目的结构化与状态阻断”上更强，但这次被 504 中断；Codex 的过程在本次任务中更短且实际闭环。
