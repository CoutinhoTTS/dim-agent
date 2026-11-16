# 多模态请求预算与图像保留

关联 [#687](https://github.com/arcships/dim/issues/687)。本文提出 ContextPreparation、ContextUsage 和模型请求边界的设计。现行契约分别见 [ContextCompaction](../plugins/ContextCompaction.md)、[ContextUsage](../plugins/ContextUsage.md)、[ModelProvider](../../architecture/rust-agent/plugins/ModelProvider.md) 和 [Aimux](../../../crates/dim-agent-aimux/README.md#工具结果中的图像)。

## 1. 目标与已知行为

每次模型调用都对实际将发送的内容检查三类限制：输入 token、请求体及图片字节数、图片张数及单图尺寸。用户图片和工具结果图片采用同一模型的图像计数规则。达到限制时优先保留当前轮用户图、最新工具批次的图和可读小字，并保持工具调用与结果配对。范围为模型输入中的静态图片，视频、PDF、音频和生成图像的计费各由其输入或生成契约负责。

Dim 已经在附件导入、`read` 和有 Session 文件存储的工具结果入口使用单图准备逻辑。常规上限为长边 2,048 px 和 5,000,000 Base64 字节。请求发送还有 32 MiB 图像编码数据及 64 MiB 最终 HTTP 请求体的本地防线。现有 WindowFacts 对原生图片每张估 1,600 token。该常数会同时高估和低估，也无法预测许多图叠加后的传输限制。 [单图准备](../../architecture/rust-agent/plugins/WorkspaceTools.md#read-results-and-images) · [发送防线](../../architecture/rust-agent/plugins/ModelProvider.md#image-handling)

旧截图另有一条文本路径。DeepChat 的旧 `cdp_send(Page.captureScreenshot)` 回执把 `{"data":"<base64>"}` 作为工具文本。用 `v0.1.0-alpha.23`（`092aaa7f`）和上游 `5f465352f` 复现时，同一张 672 × 480 PNG 通过用户图、工具图和旧 JSON 三条路径发送。原生图片在同轮工具完成后已重新计数，过阈值时先完成压缩才发续请求。旧 JSON 在 `alpha.23` 估为 110,602 token，本地测试端用 `o200k_base` 计为 296,755，超过 230,144 输入预算。原事故的 289,307 来自报告，原始请求和实际模型路由未取得。 [DeepChat 桥接改动](https://github.com/ThinkInAIXYZ/deepchat/commit/ee0fc0fc3bc112f79c90b84e99dae5673aeb97be)

## 2. Provider 调研

资料核查于 2026-11-16。按模型版本、实际端点和图像设置使用表中规则。路由组合来自 [Dim Provider drivers](../../../crates/dim-app-server/src/provider/drivers.rs) 和[内置目录](../../../crates/dim-app-server/resources/default-catalog.json)。目录的 `vision` 只能说明输入能力。计费文档、开源处理器和在线输入计数各有适用边界。运营限额可能随模型、地区或账户变化，表中的例子用于确定规则单位，运行时只采用匹配路由的已核验限制。

### 2.1 原生服务与公开计数规则

| 实际服务 | 图像如何计数 | 计数与请求限制的证据 |
| --- | --- | --- |
| OpenAI API | 按具体模型选择图块或 tile。图块型先应用该模型和 `detail` 的尺寸与图块规则，再对处理后的 `ceil(w/32) × ceil(h/32)` 乘模型系数并上取整。tile 型按 `detail` 取固定值，或应用缩放后加 tile 成本。以 `gpt-4o` 为例，其高细节规则为 85 加每个 512 px tile 的 170。解析模型特定的 `auto`，例如 `gpt-6-astra` 的 `auto` 使用 `original` 尺寸规则，某些模式会拒绝超图块上限的图。 | Responses 有 `POST /v1/responses/input_tokens`，可数同形状的图像请求。Chat Completions 用本路由的 `usage` 验证。官方 Responses Vision 页列出每次 512 MB、1,500 张和单图 30,000 图块的通用上限。模型、细节档和路由另有约束。 [图像规则](https://developers.openai.com/api/docs/guides/images-vision?api-mode=responses#calculating-costs) · [计数接口](https://developers.openai.com/api/docs/guides/token-counting) |
| Azure OpenAI | 由部署所绑定的基础模型和版本决定图像规则。Azure 部署名可由用户命名，需要可信的部署映射和该 API 的限制。 | 同一部署与协议的用量用于校验。Foundry 的 Chat 图像指南另列每次 10 张限制，不能把该例直接写入 Dim 的 Azure Responses 路由。 [Azure 图像请求与限制](https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/gpt-with-vision) |
| Anthropic Claude API | `ceil(w/28) × ceil(h/28)`。先按长边和图块预算做等比适配。官方标准档上限为 1,568 px 和 1,568 visual token，高分辨率档为 2,576 px 和 4,784 visual token。按具体模型选档。部分 computer use / browser use 工具的超尺寸截图会被拒绝，适配时要验证该工具及图像设置是否允许服务端自动缩放。 | Messages `count_tokens` 支持图片、工具和 system。图片张数会影响尺寸限制，`tool_result` 内的图片也计数。 [图像规则](https://platform.claude.com/docs/en/build-with-claude/vision) · [缩放细节](https://platform.claude.com/docs/en/build-with-claude/vision-coordinates#how-claude-resizes-and-pads-images) · [计数接口](https://platform.claude.com/docs/en/build-with-claude/token-counting) |
| Google Gemini API / Vertex AI | 早期 Gemini 文档描述小图 258 token 和大图按 768 px tile 计数。Gemini 2.5 的 `media_resolution` 资料还描述 64、256 及 Pan & Scan 的额外开销。Gemini 3 的低、中、高、极高档分别约 280、560、1,120、2,240 token。先确定该模型世代、请求 API 和实际分辨率设置，再用对应规则。 | `countTokens` 可接收多模态输入。Vertex 文档明确说明多模态预数是估计值，应与执行后的 `usage_metadata` 比对。Vertex 是独立端点，需单独确认兼容性。 [通用计数](https://ai.google.dev/gemini-api/docs/tokens) · [分辨率档](https://ai.google.dev/gemini-api/docs/generate-content/media-resolution) · [Vertex 计数](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/capabilities/get-token-count) |
| Amazon Bedrock | Converse 的请求结构由 Bedrock 定义，底层模型决定图像 token。可将同一版本的公开模型规则用作待校验的本地估算。 | `CountTokens` 需核对模型、区域和 `Converse` / `InvokeModel` 输入形态。Converse 文档对顶层消息图像列出 20 张、单图 3.75 MB、8000 px 限制，嵌套工具图片与特殊模型另核对。跨区推理的计数支持也可能不同。 [计数支持](https://docs.aws.amazon.com/bedrock/latest/userguide/count-tokens.html) · [Converse Message](https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Message.html) |
| 阿里云百炼 Qwen | 对支持的模型复现服务端的尺寸对齐、最少像素和最大像素规则。对齐后按 `w' × h' / s² + 2` 估算，`s` 由模型决定。文档列出 Qwen3-VL 等为 32、Qwen2.5-VL 等为 28。带上实际的 `max_pixels` 或 `vl_high_resolution_images`。 | 官方公式是近似值，按具体模型、实际供应端点和返回用量校验。模型别名及百炼托管的第三方模型单独匹配。 [公式和缩放](https://www.alibabacloud.com/help/en/model-studio/vision) · [设置](https://docs.modelstudio.console.alibabacloud.com/en/model-studio/qwen-api-via-openai-chat-completions) |
| DeepSeek | 公开 Vision 支持以 `deepseek-flash` 为例，服务端按尺寸缩放且公开单图 1,024 token 上界。官方有尺寸计算器，第一版可对已核验的直接路由使用上界。视觉能力按精确模型判断。 | 官方文档分别给出 48 MiB 请求体、内联单图大小、总图片大小、张数和多图尺寸约束。用 `usage` 验证图片档位。 [Vision 与限制](https://api-docs.deepseek.com/guides/vision/) · [尺寸计算器](https://api-docs.deepseek.com/quick_start/token_usage/#calculate-image-token-usage) |
| 火山方舟 / 豆包 | Seed 2.0 及后续文档给出像素面积除以 1,764 并按档位裁切的估法，也将该系列默认 `high` 描述为固定 1,280。以成功请求区分服务端实际档位和计数，未核验前可使用匹配档位的公开上界。其他模型的常数及 `detail` 另查。 | 核对指定模型的 `detail`、尺寸、Base64 单图和请求体限制，避免把常规 Seed 规则用于方舟托管的 DeepSeek 等模型。 [火山方舟图片理解](https://docs.volcengine.com/docs/ark/image-understanding?lang=zh) |

### 2.2 需要路由实测的服务

| 实际服务 | 公开证据和可采用的预算口径 |
| --- | --- |
| Moonshot / Kimi API / Kimi Code | 官方帮助页称视觉图像按每张 1,024 token 计费，未逐个绑定模型及路由。K2.6 开源权重的处理器使用尺寸、padding 与 patch merge。两者仅作为验证线索。Moonshot 提供 `estimate-token-count`，需确认其对目标模型、图片块、工具和最终格式的覆盖。Kimi Code 路由独立验证。 [官方帮助](https://www.kimi.com/en/help/kimi-api/api-model-capabilities) · [估算接口](https://platform.kimi.ai/docs/api/estimate) · [K2.6 处理器](https://huggingface.co/moonshotai/Kimi-K2.6/blob/main/media_utils.py) |
| xAI / Grok | 官方图像理解文档给出 `detail` 和图片输入限制，Chat 文档列出单图 20 MiB。未找到按模型版本可直接复用的图像 token 公式。用同路由成功用量配对验证 Chat 和 Responses，Grok Build 套餐同样单独验证。 [xAI 图像理解](https://docs.x.ai/developers/model-capabilities/images/understanding) · [Chat 限制](https://docs.x.ai/developers/model-capabilities/legacy/chat-completions) |
| 智谱 / Z.ai | GLM-4.6V 和 GLM-5V-Turbo 官方支持视觉输入，托管路由的每图公式待实测。GLM-V 开源处理器仅用于对应开源部署及已确认的设置。 [GLM-4.6V](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v) · [GLM-5V-Turbo](https://docs.bigmodel.cn/cn/guide/models/vlm/glm-5v-turbo) · [GLM-V 源码](https://github.com/zai-org/GLM-V) |
| MiniMax、Mistral | MiniMax M3 发布的处理器采用尺寸对齐及 patch merge。Mistral 提供带图像配置的官方 tokenizer，图像上限按模型与部署确认。托管 API 要用实际用量验证，开源模型需匹配权重和处理器配置。 [M3 处理器](https://huggingface.co/MiniMaxAI/MiniMax-M3/blob/main/image_processor.py) · [Mistral tokenizer](https://github.com/mistralai/mistral-common) · [Mistral 请求限制](https://docs.mistral.ai/resources/known-limitations#vision) |
| OpenRouter、AIHubMix、ZenMux、New API、Vercel AI Gateway 等中转 | 区分网关的请求限制和实际选中的上游模型版本。模型名含厂商前缀、使用相同消息字段、或连接采用 Responses，均不足以证明同一计数。网关可变路由走未知模型规则。OpenRouter 的 generation 元数据可区分上游、`native_tokens_prompt` 和 `tokens_prompt`，用于具名路由的配对验收。 [OpenRouter 路由](https://openrouter.ai/docs/guides/routing/provider-selection) · [Generation 元数据](https://openrouter.ai/docs/api/api-reference/generations/get-generation) |
| OpenAI Codex、未核验的托管套餐、自建端点和本地模型 | 使用路由明确声明的规则与限额。vLLM 可配置图片张数及处理器参数，Ollama 可返回 `prompt_eval_count`，NVIDIA NIM 的输入和限制取决于模型及部署。其他目录项（包括无公开公式的视觉模型和别名）采用同一未知路由流程。 [vLLM 部署参数](https://docs.vllm.ai/en/stable/configuration/engine_args/) · [Ollama Chat](https://docs.ollama.com/api/chat) · [NIM 多模态输入](https://docs.nvidia.com/nim/large-language-models/latest/advanced-use-cases/multimodal-input.html) |

### 2.3 可复核算例

以下是各家公开规则对同样尺寸的计算例子，图片计数以外的请求内容另算。未调用付费 API。

| 处理规则 | 672 × 480 | 2,048 × 2,048 |
| --- | ---: | ---: |
| Dim WindowFacts v1 | 1,600 | 1,600 |
| OpenAI `gpt-6-astra`，`high` | 378，即 21 × 15 × 1.2 | 3,000，适配至 1,600 × 1,600 后计 2,500 个图块 |
| Claude 标准档 | 432，即 24 × 18 | 1,521，正方形在 1,568 visual token 预算内可覆盖最多 39 × 39 个图块 |
| Gemini 3，`MEDIA_RESOLUTION_HIGH` | 约 1,120 | 约 1,120 |
| DeepSeek Flash，使用文档上界 | ≤ 1,024 | ≤ 1,024 |

## 3. 估算与请求校验

### 3.1 路由、图像数据与计数口径

Provider 的已采用模型配置提供计数规则及限制的适用身份。身份至少区分服务、可确认的具体模型版本或部署、协议、影响图像缩放的设置和规则版本。一个路由可有单图原始或编码字节、尺寸、总图片字节、最终请求字节和图片张数等限制。每项带单位和适用条件。接入已知官方规则使用经过核验的端点与模型映射。自定义配置可声明自己路由的限制。缺失的值表示未知，正无穷只用于供应商明确无该项限制时的比较语义。实际路由变化使匹配的规则和基准失效，经重新解析后再准备请求。

图像事实由已经接受该图的所有者产生。记录或取得规范图像的媒体类型、方向修正后的宽高、原始编码字节数及现有文件引用。附件／工具的既有单图准备已经得到这些信息。用文件身份复用元数据。旧文件引用在第一次真实准备需要该图时走受限、可取消的头部读取。由文件所有者验证归属、版本和读取权限。内容相同的多次出现按发送次数计数和计体积。无尺寸图按已知单图上界或兼容估算计数，文件缺失按现行错误处理。

准备阶段先确定最终候选会保留哪些原生图片、替换成哪些文本、使用哪个尺寸和细节档。ModelProvider 和 Aimux 消费同一选择，按协议确定工具图留在工具结果内、随后的用户消息或文本占位。工具图带来的说明文本和消息边框也计入候选。每个保留的图片块只产生一次视觉贡献。相同的原生图 Base64 在传输中是图像数据。旧工具回执的 Base64 属于文本贡献。

### 3.2 三项检查

以 `context window − 已采用输出预留` 计算输入额度。路由另有明确独立输入限额时，取两者较小值为输入预算 `B`。压缩阈值 `T` 应用现有预留算法和更低的用户覆盖值。按以下顺序取图像贡献，每种情况均记录依据。

1. **公式**。已核验的模型和设置使用公开公式，对最终发送尺寸应用该模型已确认的服务端缩放。模型将拒绝的图片形态直接走请求限制检查。
2. **文档上界**。适用模型有明确单图上界但缺公式、精确尺寸或分辨率开销时，使用适用上界。文档给出多个可能档位时，取有效范围内较大的已知上界。
3. **兼容估算**。未知路由且无明确上界时保留每图 1,600 token。这一估值允许继续使用未知 provider，标为启发式并按实际用量验证。保持现有压缩预留，另外执行所有已知的传输限制。对未知的服务端限制保留正常 provider 拒绝路径。

以现有 system、tools 和文本计数为基础。设 `L(R)` 是候选请求的本地估值。已有同路由、同算法、同固定内容、同已发送图像派生形态和同有效前缀的基准 `P` 时，取 `U(P) + L(R) − L(P)`，其中 `U(P)` 是 provider 报告的输入总量，缓存输入只算一次。裁掉前缀中的图、更换尺寸、压缩、修改候选的协议包裹或路由改变时，按现行基准适用证明重新判定。证明失败采用本地估值。对未知网关，只在同一上游可证明且用量口径明确时建立基准。 [基准所有权与适用条件](../../adr/023-context-usage.md#d3-服务商-usage-校准对应请求的基准)

生产默认靠本地估算和已有 `usage`。线上预数作为经配对校验的路由能力单独启用。仅对带新图、候选已经通过传输检查且估值达到 `T` 的请求，允许在本地决定压缩前向目标端点预数一次。需复用实际端点、模型、凭据域、system、tools、图像详情与消息编码。预数沿同一准备任务的期限和取消信号，一条准备／恢复链至多一次。取消使该请求停止。返回不可用或超时则回到本地政策。官方说明为近似值的预数可以提高估值，只有经过配对验证的同形状计数可以降低估值。已修改候选重新走本地检查，预数结果只对应原候选。

| 检查 | 准备阶段 | 最终发送阶段 |
| --- | --- | --- |
| 输入 token | `estimate ≥ T` 沿现行压缩及 regrowth 规则。超过 `B` 沿现有预算恢复，硬输入超限不等 regrowth。压缩后的续请求用同一规则复检。 | 只复用与实际路由和内容相符的估值。发送的内容发生变化时，按实际内容和输出预留校验。 |
| 单图／总图片字节、尺寸与张数 | 按真实保留的图片和载体累计。内联 Base64 编码长度为 `4 × ceil(raw bytes / 3)`，检查区分原图字节、编码字节和含包装的大小。先应用本地上限与已知 provider 上限中对同一指标更严的值。多图尺寸规则用候选的实际张数。 | 保留 Aimux 的图像编码防线。检查实际适配和实化后的图片形态，文件引用与内联编码分别按该路由的计量单位检查。 |
| HTTP 请求体 | 基于真实协议包含文本、工具参数、包装及图像内容得到体积。可提前从已知图像长度算内联贡献。接近已知限制时由适配器给出实际序列化体积或可靠上界。 | 针对即将发送的序列化字节执行精确上限检查，复用已经准备好的请求数据。同一请求只走一套协议转换。 |

选中的模型不支持视觉时，用既有占位符计文本与请求体，图片张数和视觉 token 为零。路由或候选身份失配时重新准备。对支持自动路由的网关，检查网关自己的限制及任何已知的共同约束，效果保证限于可确知的上限。请求拒绝应说明实际观测到的指标、数值、单位和可确认的限制。原始 Base64、凭据和图片正文留在受权限约束的内容路径，标量诊断不复制它们。

## 4. 保留、缩图与超限恢复

准备每次普通调用、同轮工具后的续调用和摘要调用时，都执行对应路由的检查。普通请求按以下顺序处理。同一次准备中按来源顺序检查图片，每个移除决定只发生一次，缩放候选按适用档位或图像准备器给出的尺寸上限递减。后续候选须减少至少一个当前超限指标，并保持其他已满足的硬限制。摘要模型用自己的限制，切片时按它的输入预算过滤或缩放图片，避免将原超限请求整体发送给摘要模型。

1. **按限制选择处理。** 单图违反明确尺寸、像素或字节约束时，复用图像准备器产生发送用副本。其他情况由整次请求是否达阈值或硬限制决定。窗口到阈值走既有上下文压缩。对总图数、图片字节或 HTTP 体积压力，先处理可回读的较早工具图。
2. **按来源顺序移出可回读的旧工具图。** 从最早的来源位置和块序号起，替换足够数量的图片块为短文本。文本携带可读取原图的现有路径或事件句柄，文字工具结果、错误状态和调用 ID 原位保留。优先使用 SessionStore 确认归属的现有文件路径，并遵守 `read` 的工具准入。可自动移出的图片来自已结束的轮次，或同一轮中已随成功模型请求发送的较早工具批次，且模型能用现有工具找回该图。Recall 当前仅显示图像标记，其所有者可以扩充回读契约。保留当前轮已采用的用户图片、最新一批工具图和既有固定上下文，直到按本节缩放或报错。
3. **按超限指标选择缩法。** 字节超限先试同尺寸的重编码，截图优先用保留小字和透明度的编码。像素或视觉 token 超限时按模型的有效尺寸或细节档等比缩小，直到可减少相应指标。图片张数靠移除或让调用者拆分输入解决。用户或工具指定像素、坐标或高细节时，保持相应要求或返回明确错误。依赖坐标的工具回执如要缩小，须有其所有者给出的坐标换算契约，能说明源尺寸和实际发送尺寸。当前轮的多个必要图片仍放不下时，报出限制和建议拆分的图数或尺寸。
4. **重新核对最终候选。** 上下文压缩成功后复查三类预算。字节或张数仍超限时执行已授权的请求内替换，再走最终检查。若没有满足限制的候选，已知超限在 HTTP 前结算。发送后 provider 明确报告图片数量、大小或输入上限，且当次尚未有已接受输出时，允许既有失败链发起一次可证明变小的准备。只对已分类的限制执行该路径。在没有更小合法候选、流已产出、取消或代际关闭时沿原终态结束。纯正文过大沿原有请求限制处理。

改变上下文的选择在准备交接前完成。请求内的替换和已选派生图进入现有 `model/call` 所对应的逻辑请求，作为实际交接的内容。供应商序列化只负责呈现这些选择。源图由原文件或持久输入／结果继续持有，缩图作用于请求使用的副本。交接时沿 [v50 图片存储](../appserver/native-storage-format.md#v50-native-image-migration) 使用 Session 文件引用。先在有界请求缓冲中选择和校验派生图，只为最终选中的派生图发布文件和属主，再提交引用它的请求事实。文件用既有 Session 生命周期管理，临时候选缓冲在任务结算时释放。重试以来源位置和本次模型配置为依据。

旧 Base64 回执由桥接生产者从已验证的截图输出产生原生图片块。旧记录兼容只在能识别生产者、具体工具方法和成功回执结构的边界验证后派生图片。一般工具文本维持文本语义，可由实际路由的计数或上下文压缩处理。 [既有图片语义](../plugins/Attachments.md) · [Recall 的工具结果](../plugins/Recall.md#34-how-each-thing-is-told)

## 5. 所有权、增量和恢复

| 所有者 | 负责的变化 |
| --- | --- |
| Provider（进程与既有 Session 模型采纳） | 从可信目录或显式连接配置提供本次模型路由及其适用限制。连接改动或实际模型版本变化形成新的适用身份。现有的认证、模型采纳和连接所有权适用。 |
| 图片生产者与 SessionStore | 复用单图准备结果，按已有文件归属提供不可变元数据和受限读取。需准备发送副本时按原文件授权读取，不进行网盘上传或跨源复用凭据。 |
| ContextUsage（分支 Capability） | 在已有相关事实到达时维护可相加统计与 provider 基准。以算法和路由身份为界，只累计同一套规则下的统计。有效前缀的文字和图片贡献继续由来源位置关联。 |
| ContextPreparation（分支 Capability） | 拥有候选选图、预算及压缩决策。复用可证明等价的统计，结合本次候选内的图像差量。把已准备的不可变请求交给现有模型调用。 |
| ModelProvider 与 Aimux | 复用现有调用及取消生命周期。Aimux 按选定路线编码和检查实际请求。需要本次请求尺寸的数据通过既有模型请求边界提供，格式及限制的错误由现有结算链递达。 |

新的可加统计以文本贡献、图片 token、图片张数及可加的图像字节贡献为主。路由特有包装和多图条件校验属于实际请求。按图片的字节长度计算预算时使用该图已选派生形态。正常追加只计新增内容，候选移除时按变化块扣除或替换，同一不变图的头部及尺寸规则可复用。确实无法证明候选与统计等价时，复用准备中已有的上下文计算一次，不把临时修改后的请求当作原分支普通贡献。与窗口估算无关的单次 HTTP 整体大小不持久化为累计上下文事实。

为新贡献结构使用新的 WindowFacts 算法标识和新的 ContextUsage 私有统计 schema。旧的 `{serialized_bytes,images}` 无法恢复尺寸分布，也无法校准新算法。读取历史业务事实使用现有格式。旧数字可在兼容的旧算法中使用，采用新计数规则时按 [ContextUsage §6.2](../plugins/ContextUsage.md#62-确实没有统计) 在下一次真实请求建立新 epoch，沿固定截止一次初始化。常规 resume 和 `session.context.usage` 保持只读数字与索引的路径。只复用 fork 可见范围内、算法及路由相容的祖先数字。未完成任务由原 owner 结算，重开 Session 时重新准备。发布模型能力和完整候选仍经现有 Timeline 交接。统计升级为派生记录的显式不兼容变更，持久层仍按 owner 的不透明数据保存。 [ADR 023](../../adr/023-context-usage.md) · [现有统计版本](../plugins/ContextUsage.md#61-统计版本与旧数据)

## 6. 实施顺序与验收

- **先核验并接入可验证的直接路由。** 选择一个 OpenAI Responses 的图块模型和一个 Claude Messages 模型，分别固定模型版本与细节设置。沿已有 WindowFacts 及 ContextUsage 增加可加贡献，复用本地发送上限。完成配对验证后再启用 Gemini 分辨率规则、百炼公式或 DeepSeek 上界。纯文本候选复用现有计数和发送流程。其余端点采用兼容估算和已知硬限额。路由数据进入现有 Provider 目录与采用机制。
- **再启用超限前的图像处理。** 先支持携带可回读路径的旧工具图及单图缩放，复用现有请求形态与限制。在近限额配对数据支持后才开启同路由线上预数。需要通过事件回读的图片由 Recall 所有者单独确定接口后再纳入。
- **用真实 provider 做配对测试。** 固定端点、模型版本、system、工具、消息和输出预留。比较空图基线与加图请求，分别采集用户图、同轮工具图和多图请求。记录上游身份、尺寸、格式与细节档、图片张数、原始／编码／请求字节、估值及依据、预数、实测输入、输出预留、阈值和实际的请求状态。图块计数用小图、方图、极端长宽比及图块边界。同尺寸 PNG/JPEG 验证体积差异。发布前要求选定路由的测试集在相同余量下没有漏拦的已知限制，并比较相对 1,600 常数的高估、低估和提前压缩次数。区分预数、输入用量和计费口径。
- **打到真实请求边界。** 用隔离 Session 和 HTTP 夹具分别让 token、总请求字节数、单图限制、张数先超限，包括 token 仍有余量时的多图场景。核对工具结果事件、同轮下一请求预算、压缩的完成事件顺序及 provider 实收候选。断言只发送符合已知限制的请求、工具 ID 成对、可回读引用有效且最新必要图片保留。将含 `{"data":"…"}` 的旧工具文本作为独立回归。每个存储或传输阈值同时验证等于上限和超一个单位。
- **核对恢复和计算开销。** 覆盖同轮切换模型、上游回退、重复或迟到的 usage、无 usage、取消准备、纯文本模型、不可访问的旧图、压缩后重载和 fork。兼容与新统计使用一次性库验证。与完全计数做等价比较，断言正常 resume、用量读取及稳定路由续请求不重复读旧图正文或针对旧图逐张预数。HTTP 返回 413 后验证同一失败链的有界处理。细节验收用含小字和坐标的真实截图，由任务正确率和读取需求判断缩放代价。

## 7. 规则检查

本提案涉及 ContextPreparation、ContextUsage 和 ModelProvider 既有分支 Capability。Provider 设置、Aimux 编码和文件读取沿现有进程／Session 部件。按 [设计规则 §1、§4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) 和 [ADR 002](../../adr/002-plugin-scope.md) 验收：

| 条款 | 应用 |
| --- | --- |
| §4.1 指令及结果 | 候选修改属于准备任务，源事件和文件由各 owner 持有。需要工具回读时沿既有调用与结果事实。 |
| §4.2 交接 | 选图和预算在交接前完成。`model/call` 对应已准备的候选。受理后的停止或失败沿原链结算。 |
| §4.3 before | 尺寸／字节读取、编码和网络预数在可取消的准备任务完成，提交回调只使用已经取得的值。 |
| §4.4 通用机制 | Timeline 和 Dispatcher 交付已有业务事实。按 provider 匹配由业务所有者实现。 |
| §4.5 批次 | 沿现有压缩和请求事实的原子写入。派生统计仅在 owner 完成对应来源后发布。 |
| §4.6 上下文来源 | 候选从已采用输入、工具结果及有效提示派生。回读原图经工具结果进入后续上下文。 |
| §4.7 依赖 | 沿请求准备、模型宿主与 `sessionFiles` 的公开边界取数，不从文件或模型 owner 的内部缓存读取。 |
| §4.8 基础设施 | 需求在既有业务边界解决。 |
| §4.9 事实 | 保留已有调用、用量、预算、压缩与终态事实，准备临时值及线上预数诊断只随当前任务。 |
| §1、ADR 002 D1–D3 | 状态仍归现有 owner，沿原生命周期启停、关闭和结算，无独立插件、接线或重载机制。 |
| ADR 002 D4–D5 | 限制和模型规则是所属目录和已采用配置的数据，跨语言依赖使用已有具名服务和语言中立的请求／响应资料。 |

### 向所有者提出的需求

以下为 [#687](https://github.com/arcships/dim/issues/687) 的业务所有者需求。现有配置能说明视觉能力与窗口大小，现有最终防线能挡住发送，但两者不足以让准备阶段识别某条路由更小的图片限制或在交接前恢复。

| 所有者 | 所需能力、为何需要、顺序与恢复 |
| --- | --- |
| Provider、ModelProvider 和 Aimux | 在同一已采用路由的准备／执行合同中传递已核验的限制和预算所需形态。适配器给准备阶段提供可证明的大小或上界，执行阶段使用同一候选检查最终尺寸。限制改变则重新准备。任务取消／重载释放准备数据和临时请求，沿既有终态结算。 |
| SessionStore、附件与工具生产者 | 已持有的文件引用目前主要提供字节读取。准入阶段需要已接受图的不可变尺寸和长度，以免按请求反复解码。优先复用已有单图准备输出、属主元数据及公开读取操作。可回读路径来自既有 Session 文件所有权，派生副本沿现有文件发布和关闭合同。引用的来源位置与归属随既有事实确定。 |
| ContextUsage、ContextPreparation | 现有统计把每张图同看成一个单位，需按适用算法保存可加贡献及证明。依 §5 版本化，沿相同来源截止、批次和 fork 可见范围恢复。 |
| Recall（需要扩大回读范围时） | 现有文本图像标记无法重新提交原图。先使用可通过已有工具打开的路径。若需要按旧事件回图，由 Recall 采用其既有 `open` 语义，按访问范围返回真实图片块并验证对应 Session 文件所有权。排序沿既有工具调用及回复。 |

删减检查保留现有计数、压缩、图片准备和发送防线各一份。三类限制有各自的失败条件，须分别检查。运行时默认使用本地估算和现有用量反馈，线上预数只作为经核验的近阈值能力。
