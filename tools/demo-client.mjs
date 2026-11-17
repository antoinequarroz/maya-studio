// Client de démonstration de l'API WebSocket (Node 22+, sans dépendance).
// Il fait ce que Nino fera : créer des scènes, placer des sources, couper,
// enregistrer quelques secondes et récupérer une capture d'écran.
//
// Usage : node tools/demo-client.mjs [ws://127.0.0.1:4460] [jeton]

import { writeFileSync } from "node:fs";

const url = process.argv[2] ?? "ws://127.0.0.1:4460";
const token = process.argv[3];
const ws = new WebSocket(url);
const pending = new Map();
let nextId = 1;

const request = (type, data) =>
  new Promise((resolve, reject) => {
    const id = String(nextId++);
    pending.set(id, { resolve, reject, type });
    ws.send(JSON.stringify({ op: "request", id, type, data }));
  });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

ws.onmessage = async (e) => {
  const msg = JSON.parse(e.data);
  if (msg.op === "hello") {
    console.log(`connecté à ${msg.app.name} ${msg.app.version} (protocole ${msg.protocol}, ${msg.capabilities.join(", ")})`);
    ws.send(JSON.stringify({
      op: "identify", name: "demo-client", token,
      app: { id: "demo", name: "Client de démo", version: "0.1.0" },
    }));
  } else if (msg.op === "identified") {
    run().catch((err) => {
      console.error("échec :", err.message);
      process.exit(1);
    });
  } else if (msg.op === "event") {
    console.log(`  ⚡ événement ${msg.app}.${msg.type}`);
  } else if (msg.op === "response") {
    const p = pending.get(msg.id);
    if (!p) return console.error("réponse sans requête :", msg);
    pending.delete(msg.id);
    msg.ok ? p.resolve(msg.data) : p.reject(new Error(`${p.type}: ${msg.error.code} (${msg.error.message})`));
  }
};

async function run() {
  console.log("requêtes :", (await request("GetRequestList")).requests.length);

  await request("CreateScene", { name: "Plateau" });
  await request("AddSource", { scene: "Plateau", name: "Fond", kind: "color", rgba: [20, 30, 60, 255] });
  await request("AddSource", {
    scene: "Plateau", name: "Incrustation", kind: "test_pattern",
    transform: { x: 760, y: 40, width: 480, height: 270 },
  });
  await request("AddSource", {
    scene: "Plateau", name: "Bandeau", kind: "color", rgba: [200, 30, 40, 255],
    transform: { x: 0, y: 600, width: 1280, height: 80, opacity: 0.85 },
  });

  const rec = await request("StartRecording", {});
  console.log("enregistrement vers", rec.target);
  await sleep(1500);

  await request("SetPreviewScene", { scene: "Plateau" });
  await request("Cut");
  await sleep(1500);

  await request("UpdateSource", { scene: "Plateau", source: "Bandeau", visible: false });
  await sleep(1000);

  const stop = await request("StopRecording");
  console.log(`enregistré : ${stop.frames} images en ${stop.duration_s.toFixed(1)} s`);

  await request("UpdateSource", { scene: "Plateau", source: "Bandeau", visible: true });
  const shot = await request("GetScreenshot", { format: "png" });
  writeFileSync("capture-programme.png", Buffer.from(shot.image.split(",")[1], "base64"));
  console.log("capture : capture-programme.png");

  try {
    await request("RemoveScene", { scene: "Plateau" });
  } catch (err) {
    console.log("erreur attendue :", err.message);
  }
  ws.close();
}
