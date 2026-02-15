# Provider Connect & Retry Config Design

## 1. Worker Max Retries Config
- Ctrl+P → Main Menu → "Max Retries" → 数字列表 (1-10)
- 更新 App.default_max_iterations, 同步到 Engine

## 2. Connect Provider
- 预定义 6 个模板: Anthropic, OpenAI, GitHub Copilot, OpenRouter, Google Gemini, DeepSeek
- 流程: Ctrl+P → Connect Provider → 选模板 → 输入 API Key → 保存 DB
- PopupMenu 新增: ConnectProvider, ProviderApiKeyInput
- 沿用现有 api_format 体系
