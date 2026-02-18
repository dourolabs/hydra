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
  const [buttonVariant, setButtonVariant] = useState<"primary" | "secondary" | "ghost">("primary");
  const [buttonSize, setButtonSize] = useState<"sm" | "md" | "lg">("md");

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
              ]}
              onChange={(e) =>
                setButtonVariant(e.target.value as "primary" | "secondary" | "ghost")
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
