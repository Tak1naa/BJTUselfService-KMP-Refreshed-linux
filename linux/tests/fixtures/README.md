这些是虚构数据，按完整 Kotlin 源码的解析器测试和 HTML 结构整理，包含真实中文规则。
它们不是本轮从学校生产系统抓取的页面，不能证明当前线上协议兼容。
`teachers/courses/selection/grades` 对照 `shared/src/commonTest/.../*HtmlParserTest.kt`。
SM2 测试向量来自原仓库 `SsoSm2Test.kt` 的专用测试密钥，不是用户密钥。
新增真实页面时，先脱敏并人工核对；禁止将个人身份、成绩、Cookie 或隐藏会话字段放入 fixture。
