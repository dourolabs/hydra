import { UI_VERSION } from "@metis/ui";

export default function App() {
  return (
    <div
      style={{
        backgroundColor: "#0a0a0a",
        color: "#e0e0e0",
        fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
        minHeight: "100vh",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <h1 style={{ color: "#00cc66" }}>metis</h1>
      <p>Web interface — coming soon</p>
      <p style={{ color: "#555555", fontSize: "0.8rem" }}>@metis/ui v{UI_VERSION}</p>
    </div>
  );
}
