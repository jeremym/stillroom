import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [status, setStatus] = useState<string>("checking…");

  useEffect(() => {
    invoke<string>("runtime_status")
      .then(setStatus)
      .catch((e) => setStatus(`error: ${String(e)}`));
  }, []);

  return (
    <main className="shell">
      <h1>Stillroom</h1>
      <p className="tagline">Describe the goal. Get the result. Open the door.</p>
      <p className="status">Runtime: {status}</p>
    </main>
  );
}

export default App;
