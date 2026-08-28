import { CheckCircle2, RefreshCw, ShieldAlert, ShieldCheck, TriangleAlert } from "lucide-react";
import type { ApiClient } from "../../api/client";
import { Badge, Button, Panel } from "../ui";
import { trustSignals } from "./model";
import { useEnterpriseStore } from "./store";

export function TrustPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const signals = trustSignals(snapshot);
  return (
    <div className="enterprise-page enterprise-trust">
      <Panel
        title="Trust center / 信任中心"
        actions={<Button aria-label="刷新信任状态" onClick={() => void store.load(true)} size="compact" variant="quiet"><RefreshCw aria-hidden="true" size={14} />刷新</Button>}
      >
        <p className="enterprise-page__lede">
          聚合 Connection 健康、运行失败、HumanTask 和未激活草稿。执行权限仍由不可变 Flow Revision 与调用时 live gate 决定，本页不替代授权边界。
        </p>
        <ol className="enterprise-trust-signals">
          {signals.map((signal) => {
            const Icon = signal.level === "healthy" ? CheckCircle2 : signal.level === "warning" ? ShieldAlert : TriangleAlert;
            return (
              <li className={`is-${signal.level}`} key={signal.id}>
                <Icon aria-hidden="true" size={18} />
                <span><strong>{signal.title}</strong><small>{signal.detail}</small></span>
                <Badge variant={signal.level === "healthy" ? "success" : signal.level === "warning" ? "danger" : "warning"}>{signal.level}</Badge>
              </li>
            );
          })}
        </ol>
      </Panel>
      <Panel title="Execution invariants / 执行不变量">
        <ul className="enterprise-invariants">
          <li><ShieldCheck aria-hidden="true" size={16} /><span><strong>Immutable identity / 不可变身份</strong><small>每个 Workflow Agent 节点固定模板版本、content hash 与 Connection 操作。</small></span></li>
          <li><ShieldCheck aria-hidden="true" size={16} /><span><strong>Least privilege / 最小权限</strong><small>节点权限只能从 Agent 配置与 Flow Revision 逐层收窄，不能从 Thread MCP 状态扩权。</small></span></li>
          <li><ShieldCheck aria-hidden="true" size={16} /><span><strong>Durable control points / 持久化控制点</strong><small>审批、补输入、重连、效果核对和输出审查统一形成 HumanTask。</small></span></li>
          <li><ShieldCheck aria-hidden="true" size={16} /><span><strong>Fail closed / 失败关闭</strong><small>认证过期、能力移除、描述变更或快照缺失都会在外部调用前拒绝。</small></span></li>
        </ul>
      </Panel>
    </div>
  );
}
