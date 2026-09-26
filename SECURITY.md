# Security policy

## Supported versions

ThoughtsFlow is an early beta. Security fixes target the latest published beta and the `main` branch. Older beta versions do not have a separate maintenance or backport commitment.

## Reporting a vulnerability

Please use GitHub's private [Report a vulnerability](https://github.com/CollaalloC/ThoughtsFlow/security/advisories/new) form. Include the affected version, operating system, reproduction steps, likely impact, and a minimal demonstration using test data.

Do not post API keys, private workspaces, real user content, or exploitable details in a public issue. If GitHub does not offer private reporting, open an issue containing only a request for a private contact channel. Do not include vulnerability details until that channel is available.

This is a community project without a guaranteed response time. A report is not consent to test third-party providers or other users' systems.

## Current security boundaries

- **Local storage is not encrypted by the app.** SQLite stores workspace content, model responses, provider settings and operation records. Markdown exports contain selected workspace content. Use operating-system access controls, disk encryption and appropriate backups.
- **Model calls can leave the device.** Selected context is sent to the configured provider. Review the endpoint and context before sending; local-first storage does not imply offline inference.
- **Credentials are session-only.** Keys briefly exist in the password input before handoff and then in Rust process memory. They are not intentionally persisted in SQLite, receipts or exports. Exiting the app clears the session; this is not a defense against a compromised operating system or process-memory inspection.
- **Endpoint restrictions are bounded safeguards.** Remote provider URLs require HTTPS; HTTP is allowed for loopback hosts. Provider requests do not follow redirects. Exact known credential strings are redacted from provider responses, but transformed or encoded secrets are not guaranteed to be detected.
- **Agent tools have external authority.** Orca and OMP are separately installed and configured. Their tools run with the permissions granted to those runtimes. A Git worktree is not a security sandbox. Review tool approvals in the native runtime and use a suitable isolated environment for untrusted tasks.
- **No automatic replay of uncertain Agent operations.** A lost response does not prove a failed dispatch. Inspect operation evidence and runtime state before trying again.
- **Test capabilities are separate from normal builds.** WebDriver and intentional failure injection require the `webview-e2e` test feature; do not distribute those builds as production installers.
- **Early installers are unsigned test builds.** Verify the repository, release version and published checksums. Checksums detect file changes; they do not replace platform signing or notarization.

Do not place secrets in source files, issue attachments or bug-report logs. See [README.md](README.md) for data handling and [CONTRIBUTING.md](CONTRIBUTING.md) for testing practices.

## 中文说明

安全漏洞请通过仓库的私密漏洞报告入口提交，并使用测试数据复现。若入口不可用，只公开请求私密联系方式，不公开可利用细节。

本项目当前是早期测试版。数据库和导出文件没有应用级加密，远程模型会收到选定的上下文，外部 Agent 具有其运行环境授予的权限。请核验发布包来源及校验和，并按工作内容选择合适的备份、磁盘加密和 Agent 隔离环境。
