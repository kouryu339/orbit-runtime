# v0.5.5 发布验证

Windows、Linux、WASM 和 Svelte 的构建源码提交为 c4439d1ad60023accba1f3f5613c3d7299f3fdd3。后续发布提交固定 SDK 下载校验值并记录验证结果。

- Rust 工作区 1041 项测试通过，36 项沿用忽略设置。格式检查通过。
- Lit 93 项与 Svelte 59 项通过。Svelte 类型检查及独立 JS 挂载、链接拦截、发送和销毁验证通过。
- Chrome Worker 15 项检查通过，包括异步工作流、宿主工具、审批、暂停继续和 IndexedDB。
- Windows DLL 和 Ubuntu WSL 中的 Linux SO 均实际加载。两个平台均验证 0.5.5、ABI major 1、先订阅、执行结果和超限终态。
- Go SDK 编译、C++17 宿主示例语法检查与 Python SDK 加载通过。
- 原生库、头文件、Schema、WASM manifest、ZIP/TGZ、独立 JS 和 SDK 下载校验值通过逐文件核对。

| 附件 | 字节 | SHA-256 |
| --- | ---: | --- |
| orbit-runtime-browser-v0.5.5.tgz | 2780200 | c3456c57727d33dc242bdbc645162c3822231b17061e253ad908b7da41f11df1 |
| orbit-runtime-browser-v0.5.5.zip | 2825780 | 9f9344044060185ceed923af6043a5457d5970ffd573161223d1f52f65fa5000 |
| orbit-runtime-frontend-svelte-v0.5.5.js | 7002896 | 3f52aba83ac258464a2e8bd31b6e50854eaec910e246223906d74c059bb97d7b |
| orbit-runtime-frontend-svelte-v0.5.5.zip | 2281602 | f053580d305ba2948dad1cfbed302ddd3d5e2edf6709d7f60fee70aa8187a551 |
| orbit-runtime-runtime-v0.5.5-linux-x86_64.zip | 15295561 | db3bdfeb305281eeac54af726a4f2740cf0e6c0a47ddd91a4f8e4253bcb01073 |
| orbit-runtime-runtime-v0.5.5-windows-x86_64.zip | 15827247 | a7a38f8a7fe98e14057835af484aaee5c05c772fac96dd0bd8d10a0ffeaad8bd |
| orbit-runtime-sdk-v0.5.5.zip | 114454 | f5f1854ee6965a84927c2756dab634b936a485a96ecca732d8a12d6e38a1caa9 |
