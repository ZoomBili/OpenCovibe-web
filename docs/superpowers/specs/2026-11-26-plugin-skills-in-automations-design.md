# 自动化技能下拉支持已启用插件的技能

日期：2026-11-26
状态：待批准

## 背景与目标

设置 → 自动化的技能下拉目前只扫描两个目录（`src-tauri/src/storage/plugins.rs:340`）：

- `~/.claude/skills/*/SKILL.md`（user）
- `{cwd}/.claude/skills/*/SKILL.md`（project）

插件提供的技能（例：superpowers 的 15 个）位于
`~/.claude/plugins/cache/{marketplace}/{plugin}/{version}/skills/`，从不出现。

目标：**只**列出已启用插件的技能，且选中后自动化能真正触发它。

## 关键约束（已在本机验证）

- CLI 的 `system/init` 把插件技能列为 `superpowers:brainstorming` 这种带命名空间的名字；
  项目/用户技能是裸名（`update-config`）。
- 自动化 runner 发送 `/{skill_name}`（`src-tauri/src/automation/runner.rs:33`），
  所以返回的 `name` 必须是 `插件名:技能名`，否则技能不会被触发。
- 插件升级会改变 installPath 的版本段，路径必须从
  `~/.claude/plugins/installed_plugins.json` 读，不能硬编码 cache 布局。
- 配置缺失或损坏时不得影响现有技能列表。

## 方案（A，已选定）

### 取数

1. 合并以下文件的 `enabledPlugins`（后者覆盖前者，只有值为 `true` 才算启用）：
   - `~/.claude/settings.json`
   - `~/.claude/settings.local.json`
   - `{cwd}/.claude/settings.json`
   - `{cwd}/.claude/settings.local.json`
2. 读 `~/.claude/plugins/installed_plugins.json`（`version: 2`），键为
   `name@marketplace`，值为安装条目数组。对每个启用键，选第一条满足：
   installPath 存在，且条目无 `projectPath` 或 `projectPath == cwd`。
3. 插件名 = 键中 `@` 之前的部分。

### 扫描

扫 `<installPath>/skills/*/SKILL.md`（复用 `scan_skills_dir` 的规则）：

- `name` = `{插件名}:{frontmatter name}`，frontmatter 缺 name 时用目录名
- `scope` = `"plugin"`
- `agent` = `"claude"`，`enabled` = `true`
- `can_edit` / `can_delete` / `can_toggle` = `false`（插件托管，应用不提供编辑）
- `description` 取自 frontmatter

### 合并与去重

顺序 user → project → plugin；插件技能若与已列出的同名技能冲突则跳过；user/project 之间维持现状（不去重，避免改变既有行为）；同一插件键只处理一次。

### 代码结构（为可测试性拆分）

- `resolve_enabled_plugin_dirs(settings_paths: &[PathBuf], installed_plugins_path: &Path, cwd: &str) -> Vec<(String, PathBuf)>`：纯函数，可注入临时路径。
- `list_standalone_skills_with_roots(cwd, user_dir, project_dir, plugin_dirs)`：扫描 + 合并。
- `list_standalone_skills(cwd)`：只负责计算默认路径并调用上面两个。

### 非目标

- Codex 插件技能
- 编辑/删除/禁用插件技能
- 前端改动（下拉已显示 `名称 · scope`，会显示 `superpowers:brainstorming · plugin`）
- 结果缓存（维持现状，每次现读）

## 测试

Rust 单测（tempdir）：

- 插件技能被列出，名字带前缀，`scope=plugin`，`can_edit=false`
- 未启用、未安装、installPath 不存在 → 不列出
- 配置文件缺失 / JSON 损坏 → 插件技能为空，用户/项目技能不受影响
- 项目级 `enabledPlugins` 覆盖用户级（true → false）
- project-scope 安装条目 `projectPath` 与 cwd 不符时跳过
- 同名技能按 user → project → plugin 优先
- 扫描只认含 `SKILL.md` 的子目录

## 风险

- 命名空间后缀取自 frontmatter `name`；本机 superpowers 与目录名一致，且 CLI 同源，风险低。
- `enabledPlugins` 出现非 `true` 的值一律视为未启用，宁可少列。

## 验证方式

- `cargo test --locked --manifest-path src-tauri/Cargo.toml list_standalone_skills` 等定向测试
- `cargo clippy` / `cargo fmt --check`
- （可选，需批准）重新构建并重启 9476 服务后，用 WebSocket 调 `list_standalone_skills` 确认返回 `superpowers:*`
