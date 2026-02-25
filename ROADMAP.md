# Legion Roadmap

> "A legion doesn't march in single file — it advances on a broad front."

This roadmap outlines the vision and development plan for Legion, a multi-agent orchestration tool for Claude Code.

---

## Vision

Turn one AI coding assistant into a coordinated squad. While Claude Code is powerful, it's limited by context window and serial execution. Legion solves this by parallelizing work across multiple AI agents, each working in isolated git worktrees, with automatic merging and quality gates.

---

## Upcoming Plans

### Plan 1: Agent Teams Full Coverage

**Goal**: Complete coverage of agent teams in Leader + full visibility and interactivity in panel

- [ ] Leader agent teams subdivision (Tech Lead, Engineer, QA, etc.)
- [ ] Visible team structure in TUI panel
- [ ] Interactive team member management
- [ ] Each team member can proxy to different models
- [ ] Team member configuration (provider, model per member)
- [ ] Real-time team status display
- [ ] Team communication in TUI

### Plan 2: Global MCP & Skill Injection

**Goal**: Unified MCP tools and skills available to all agents

- [ ] Global MCP registry (add/remove/manage)
- [ ] Inject MCP to both Leader and Workers
- [ ] Global skill system
- [ ] Skill visibility control (per-agent or global)
- [ ] MCP/Skill enable/disable toggle
- [ ] MCP credential management
- [ ] Skill parameter configuration

### Plan 3: Remote Coding

**Goal**: Control Legion from anywhere via phone/voice

- [ ] Mobile-friendly web interface
- [ ] Voice message input (speech-to-text)
- [ ] Task creation via voice
- [ ] Task status monitoring
- [ ] Code review via mobile
- [ ] Interactive chat with Leader
- [ ] Push notifications for task updates
- [ ] Secure remote authentication

---

## Milestones

### v0.2.0 — Multi-Session Persistence
*Planned: Q2 2026*

- [ ] Save/restore full session state (tickets, progress, logs)
- [ ] Session history browser
- [ ] Branch-based session switching
- [ ] Export/import sessions

### v0.3.0 — Enhanced Quality Gates
*Planned: Q2-Q3 2026*

- [ ] Configurable checkpoint commands (build, test, lint, custom)
- [ ] Checkpoint result caching
- [ ] Rollback on checkpoint failure
- [ ] Test coverage integration

### v0.4.0 — Advanced Team Workflows
*Planned: Q3 2026*

- [ ] Custom team templates (Code Review, QA, Security, etc.)
- [ ] Team communication channels
- [ ] Cross-team ticket dependencies
- [ ] Team performance metrics

### v1.0.0 — Production Ready
*Planned: Q4 2026*

- [ ] Stable API
- [ ] Comprehensive test suite
- [ ] Documentation site
- [ ] Plugin system for custom tools
- [ ] Cloud sync (optional)

---

## Current Focus

### v0.1.x — Stability & Compatibility

- [x] Tool call ordering fix for OpenAI-compatible APIs
- [x] OpenAI strict mode support
- [x] Provider switching without restart
- [x] Task board viewport improvements
- [ ] Cross-provider model matrix
- [ ] Unified error handling

---

## Feature Requests

Have an idea? Open an issue!

- 🐛 **Bug reports**: Help us fix what's broken
- 💡 **Feature requests**: Tell us what you need
- 💬 **Discussions**: Talk through ideas before filing

---

## Contributing

1. Fork the repo
2. Create a feature branch
3. Make your changes
4. Submit a PR

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

---

## Resources

- [Documentation](docs/)
- [Chinese README](docs/README_CN.md)
- [CLI Commands](README.md#cli-commands)
- [Provider Setup](README.md#provider-support)
