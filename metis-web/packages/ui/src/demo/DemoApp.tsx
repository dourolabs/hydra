import { useState } from "react";
import { Button } from "../components/Button";
import { Input } from "../components/Input";
import { Textarea } from "../components/Textarea";
import { Badge, type BadgeStatus } from "../components/Badge";
import { Select } from "../components/Select";
import { Spinner } from "../components/Spinner";
import { Panel } from "../components/Panel";
import { TreeView, type TreeNode } from "../components/TreeView";
import { Tabs } from "../components/Tabs";
import { Modal } from "../components/Modal";
import { Tooltip } from "../components/Tooltip";
import { Avatar } from "../components/Avatar";
import { LogViewer } from "../components/LogViewer";
import { Toast, type ToastVariant } from "../components/Toast";
import { JobStatusIndicator, type JobSummary } from "../components/JobStatusIndicator";
import styles from "./DemoApp.module.css";

const statuses: BadgeStatus[] = [
  "open",
  "in-progress",
  "closed",
  "failed",
  "dropped",
  "blocked",
  "rejected",
];

const sampleTree: TreeNode[] = [
  {
    id: "i-root1",
    label: (
      <span className={styles.treeLabel}>
        <Badge status="in-progress" /> i-abc123 Build web UI
      </span>
    ),
    children: [
      {
        id: "i-child1",
        label: (
          <span className={styles.treeLabel}>
            <Badge status="closed" /> i-def456 Component library
          </span>
        ),
      },
      {
        id: "i-child2",
        label: (
          <span className={styles.treeLabel}>
            <Badge status="open" /> i-ghi789 Backend proxy
          </span>
        ),
        children: [
          {
            id: "i-grandchild1",
            label: (
              <span className={styles.treeLabel}>
                <Badge status="blocked" /> i-jkl012 Auth middleware
              </span>
            ),
          },
        ],
      },
    ],
  },
  {
    id: "i-root2",
    label: (
      <span className={styles.treeLabel}>
        <Badge status="open" /> i-mno345 Fix auth bug
      </span>
    ),
  },
];

const jobsMixed: JobSummary[] = [
  { jobId: "t-aaa001", status: "complete", startTime: "2026-02-20T08:00:00Z", endTime: "2026-02-20T08:02:30Z" },
  { jobId: "t-aaa002", status: "failed", startTime: "2026-02-20T08:10:00Z", endTime: "2026-02-20T08:11:15Z" },
  { jobId: "t-aaa003", status: "complete", startTime: "2026-02-20T08:20:00Z", endTime: "2026-02-20T08:25:00Z" },
  { jobId: "t-aaa004", status: "complete", startTime: "2026-02-20T09:00:00Z", endTime: "2026-02-20T09:03:00Z" },
  { jobId: "t-aaa005", status: "running", startTime: new Date(Date.now() - 165000).toISOString() },
];

const jobsRunningOnly: JobSummary[] = [
  { jobId: "t-bbb001", status: "running", startTime: new Date(Date.now() - 42000).toISOString() },
];

const jobsAllComplete: JobSummary[] = [
  { jobId: "t-ccc001", status: "complete", startTime: "2026-02-20T06:00:00Z", endTime: "2026-02-20T06:01:00Z" },
  { jobId: "t-ccc002", status: "complete", startTime: "2026-02-20T07:00:00Z", endTime: "2026-02-20T07:02:00Z" },
  { jobId: "t-ccc003", status: "complete", startTime: "2026-02-20T08:00:00Z", endTime: "2026-02-20T08:01:30Z" },
];

const jobsMany: JobSummary[] = Array.from({ length: 15 }, (_, i) => ({
  jobId: `t-ddd${String(i + 1).padStart(3, "0")}`,
  status: (i === 3 || i === 7 ? "failed" : "complete") as JobSummary["status"],
  startTime: `2026-02-20T${String(6 + i).padStart(2, "0")}:00:00Z`,
  endTime: `2026-02-20T${String(6 + i).padStart(2, "0")}:02:00Z`,
}));

const jobsPending: JobSummary[] = [
  { jobId: "t-eee001", status: "complete", startTime: "2026-02-20T08:00:00Z", endTime: "2026-02-20T08:01:00Z" },
  { jobId: "t-eee002", status: "created" },
  { jobId: "t-eee003", status: "pending" },
];

const sampleLogs = [
  "Cloning repository dourolabs/metis...",
  "Cloned to /tmp/metis-work-abc123",
  "\x1b[32m[ok]\x1b[0m Repository cloned successfully",
  "Running cargo check --workspace",
  "\x1b[33m   Compiling\x1b[0m metis-common v0.1.0",
  "\x1b[33m   Compiling\x1b[0m metis-server v0.1.0",
  "\x1b[33m   Compiling\x1b[0m metis v0.1.0",
  "\x1b[32m    Finished\x1b[0m dev [unoptimized + debuginfo] target(s)",
  "Running cargo test --workspace",
  "running 42 tests",
  "test metis_common::tests::test_issue_id ... \x1b[32mok\x1b[0m",
  "test metis_common::tests::test_job_status ... \x1b[32mok\x1b[0m",
  "test metis_server::routes::test_create_issue ... \x1b[32mok\x1b[0m",
  "test metis_server::routes::test_list_issues ... \x1b[32mok\x1b[0m",
  "\x1b[31mtest metis_server::routes::test_delete_issue ... FAILED\x1b[0m",
  "",
  "failures:",
  "    metis_server::routes::test_delete_issue",
  "\x1b[31m\x1b[1mtest result: FAILED.\x1b[0m 41 passed; 1 failed; 0 ignored",
];

export function DemoApp() {
  const [inputVal, setInputVal] = useState("");
  const [textareaVal, setTextareaVal] = useState("");
  const [selectVal, setSelectVal] = useState("open");
  const [activeTab, setActiveTab] = useState("buttons");
  const [modalOpen, setModalOpen] = useState(false);
  const [buttonVariant, setButtonVariant] = useState<"primary" | "secondary" | "ghost" | "danger">("primary");
  const [buttonSize, setButtonSize] = useState<"sm" | "md" | "lg">("md");
  const [toasts, setToasts] = useState<{ id: number; variant: ToastVariant; message: string }[]>([]);
  const [toastCounter, setToastCounter] = useState(0);

  const addToast = (variant: ToastVariant, message: string) => {
    const id = toastCounter;
    setToastCounter((c) => c + 1);
    setToasts((prev) => [...prev, { id, variant, message }]);
  };

  const removeToast = (id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  };

  return (
    <div className={styles.app}>
      <header className={styles.header}>
        <h1 className={styles.title}>@metis/ui</h1>
        <span className={styles.subtitle}>Component Library Demo</span>
      </header>

      <main className={styles.main}>
        {/* Buttons */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Button</h2>
          <div className={styles.controls}>
            <Select
              label="Variant"
              value={buttonVariant}
              options={[
                { value: "primary", label: "Primary" },
                { value: "secondary", label: "Secondary" },
                { value: "ghost", label: "Ghost" },
                { value: "danger", label: "Danger" },
              ]}
              onChange={(e) =>
                setButtonVariant(e.target.value as "primary" | "secondary" | "ghost" | "danger")
              }
            />
            <Select
              label="Size"
              value={buttonSize}
              options={[
                { value: "sm", label: "Small" },
                { value: "md", label: "Medium" },
                { value: "lg", label: "Large" },
              ]}
              onChange={(e) => setButtonSize(e.target.value as "sm" | "md" | "lg")}
            />
          </div>
          <div className={styles.preview}>
            <Button variant={buttonVariant} size={buttonSize}>
              Active
            </Button>
            <Button variant={buttonVariant} size={buttonSize} disabled>
              Disabled
            </Button>
          </div>
          <div className={styles.preview}>
            <Button variant="primary">Primary</Button>
            <Button variant="secondary">Secondary</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="danger">Danger</Button>
          </div>
        </section>

        {/* Input */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Input</h2>
          <div className={styles.grid}>
            <Input
              label="Default"
              placeholder="Type something..."
              value={inputVal}
              onChange={(e) => setInputVal(e.target.value)}
            />
            <Input label="With error" placeholder="Invalid input" error="This field is required" />
            <Input label="Disabled" placeholder="Cannot edit" disabled />
          </div>
        </section>

        {/* Textarea */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Textarea</h2>
          <div className={styles.grid}>
            <Textarea
              label="Description"
              placeholder="Enter a description..."
              value={textareaVal}
              onChange={(e) => setTextareaVal(e.target.value)}
            />
            <Textarea label="With error" placeholder="Invalid" error="Description is too short" />
          </div>
        </section>

        {/* Badge */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Badge</h2>
          <div className={styles.preview}>
            {statuses.map((status) => (
              <Badge key={status} status={status} />
            ))}
          </div>
        </section>

        {/* Select */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Select</h2>
          <div className={styles.grid}>
            <Select
              label="Status"
              value={selectVal}
              options={statuses.map((s) => ({ value: s, label: s }))}
              onChange={(e) => setSelectVal(e.target.value)}
            />
            <Select
              label="With placeholder"
              options={[
                { value: "task", label: "Task" },
                { value: "bug", label: "Bug" },
              ]}
              placeholder="Select a type..."
              defaultValue=""
            />
          </div>
        </section>

        {/* Spinner */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Spinner</h2>
          <div className={styles.preview}>
            <span className={styles.label}>Small:</span>
            <Spinner size="sm" />
            <span className={styles.label}>Medium:</span>
            <Spinner size="md" />
            <span className={styles.label}>Large:</span>
            <Spinner size="lg" />
          </div>
        </section>

        {/* Panel */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Panel</h2>
          <div className={styles.grid}>
            <Panel header="Issue Details">
              <p style={{ color: "var(--color-text-secondary)" }}>
                Panel with a header and body content. Used for containing sections of the UI.
              </p>
            </Panel>
            <Panel>
              <p style={{ color: "var(--color-text-secondary)" }}>Panel without a header.</p>
            </Panel>
          </div>
        </section>

        {/* Tabs */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Tabs</h2>
          <Tabs
            tabs={[
              { id: "buttons", label: "Buttons" },
              { id: "inputs", label: "Inputs" },
              { id: "display", label: "Display" },
            ]}
            activeTab={activeTab}
            onTabChange={setActiveTab}
          />
          <Panel>
            <p style={{ color: "var(--color-text-secondary)" }}>
              Active tab: <strong style={{ color: "var(--color-accent)" }}>{activeTab}</strong>
            </p>
          </Panel>
        </section>

        {/* TreeView */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>TreeView</h2>
          <Panel header="Issues">
            <TreeView nodes={sampleTree} onNodeClick={(id) => console.log("clicked:", id)} />
          </Panel>
        </section>

        {/* Modal */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Modal</h2>
          <Button onClick={() => setModalOpen(true)}>Open Modal</Button>
          <Modal open={modalOpen} onClose={() => setModalOpen(false)} title="Confirm Action">
            <p style={{ color: "var(--color-text-secondary)", marginBottom: "var(--space-4)" }}>
              Are you sure you want to perform this action? This cannot be undone.
            </p>
            <div style={{ display: "flex", gap: "var(--space-2)", justifyContent: "flex-end" }}>
              <Button variant="ghost" onClick={() => setModalOpen(false)}>
                Cancel
              </Button>
              <Button onClick={() => setModalOpen(false)}>Confirm</Button>
            </div>
          </Modal>
        </section>

        {/* Tooltip */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Tooltip</h2>
          <div className={styles.preview}>
            <Tooltip content="Shown above" position="top">
              <Button variant="secondary">Top</Button>
            </Tooltip>
            <Tooltip content="Shown below" position="bottom">
              <Button variant="secondary">Bottom</Button>
            </Tooltip>
            <Tooltip content="Shown left" position="left">
              <Button variant="secondary">Left</Button>
            </Tooltip>
            <Tooltip content="Shown right" position="right">
              <Button variant="secondary">Right</Button>
            </Tooltip>
          </div>
        </section>

        {/* Avatar */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Avatar</h2>
          <div className={styles.preview}>
            <Avatar name="Alice Smith" size="sm" />
            <Avatar name="Bob Jones" size="md" />
            <Avatar name="Charlie Brown" size="lg" />
            <Avatar name="Diana Prince" size="md" />
            <Avatar name="jayantk" size="md" />
            <Avatar name="swe" size="md" />
          </div>
        </section>

        {/* Toast */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Toast</h2>
          <div className={styles.preview}>
            <Toast variant="success" message="Issue i-abc123 created successfully" duration={0} />
            <Toast variant="error" message="Failed to create issue: unauthorized" duration={0} />
            <Toast variant="info" message="Issue status updated" duration={0} />
          </div>
          <div className={styles.preview}>
            <Button
              variant="primary"
              size="sm"
              onClick={() => addToast("success", "Issue created successfully")}
            >
              Success toast
            </Button>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => addToast("error", "Something went wrong")}
            >
              Error toast
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => addToast("info", "Status updated")}
            >
              Info toast
            </Button>
          </div>
          <div
            style={{
              position: "fixed",
              bottom: "var(--space-4)",
              right: "var(--space-4)",
              display: "flex",
              flexDirection: "column",
              gap: "var(--space-2)",
              zIndex: 1000,
              pointerEvents: "none",
            }}
          >
            {toasts.map((t) => (
              <Toast
                key={t.id}
                variant={t.variant}
                message={t.message}
                onClose={() => removeToast(t.id)}
              />
            ))}
          </div>
        </section>

        {/* JobStatusIndicator */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>JobStatusIndicator</h2>
          <div className={styles.grid}>
            <Panel header="Mixed (complete, failed, running)">
              <JobStatusIndicator
                jobs={jobsMixed}
                onJobClick={(id) => console.log("job clicked:", id)}
              />
            </Panel>
            <Panel header="Single running job">
              <JobStatusIndicator
                jobs={jobsRunningOnly}
                onJobClick={(id) => console.log("job clicked:", id)}
              />
            </Panel>
            <Panel header="All complete">
              <JobStatusIndicator
                jobs={jobsAllComplete}
                onJobClick={(id) => console.log("job clicked:", id)}
              />
            </Panel>
            <Panel header="Many jobs (truncated)">
              <JobStatusIndicator
                jobs={jobsMany}
                onJobClick={(id) => console.log("job clicked:", id)}
              />
            </Panel>
            <Panel header="With pending/created">
              <JobStatusIndicator
                jobs={jobsPending}
                onJobClick={(id) => console.log("job clicked:", id)}
              />
            </Panel>
            <Panel header="No jobs (empty)">
              <JobStatusIndicator jobs={[]} />
            </Panel>
          </div>
        </section>

        {/* LogViewer */}
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>LogViewer</h2>
          <LogViewer lines={sampleLogs} />
        </section>
      </main>

      <footer className={styles.footer}>
        <span className={styles.footerText}>@metis/ui v0.0.1 &middot; Dark Terminal Theme</span>
      </footer>
    </div>
  );
}
