// Client de démonstration de l'API WebSocket (Node 22+, sans dépendance).
// Il fait ce que Nino fera : créer des scènes, placer des sources, couper,
// enregistrer quelques secondes et récupérer une capture d'écran.
//
// Usage : node tools/demo-client.mjs [ws://127.0.0.1:4460] [jeton] [mp4|mkv] [url-de-diffusion]
//                                    [--media chemin/vers/clip.mp4]
//   ex. : node tools/demo-client.mjs ws://127.0.0.1:4460 "" mp4 "srt://127.0.0.1:9000"
//   --media ajoute le fichier en source media_file (en boucle, image et son) et affiche
//   les VU-mètres (événements InputLevels).

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

const args = process.argv.slice(2);
let mediaPath;
const mediaFlag = args.findIndex((a) => a === "--media" || a.startsWith("--media="));
if (mediaFlag >= 0) {
  const [flag] = args.splice(mediaFlag, 1);
  mediaPath = flag.includes("=") ? flag.slice(flag.indexOf("=") + 1) : args.splice(mediaFlag, 1)[0];
  if (!mediaPath) {
    console.error("--media attend un chemin de fichier");
    process.exit(2);
  }
  // Le chemin est résolu par studiod : on lui passe un chemin absolu.
  mediaPath = resolve(mediaPath);
}
const url = args[0] ?? "ws://127.0.0.1:4460";
const token = args[1] || undefined;
const format = args[2] || "mp4";
const streamUrl = args[3];
let lastLevels = 0;
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
  } else if (msg.op === "event" && msg.type === "InputLevels") {
    // ~10 par seconde : on n'en affiche qu'un par seconde.
    if (Date.now() - lastLevels < 1000) return;
    lastLevels = Date.now();
    for (const s of msg.data.sources) {
      const state = s.muted ? " (coupé)" : s.audible ? "" : " (hors antenne)";
      console.log(`  ♪ niveau ${s.source_id.slice(0, 8)} : crête ${s.peak_db.join(" / ")} dBFS, RMS ${s.rms_db.join(" / ")} dBFS${state}`);
    }
  } else if (msg.op === "event") {
    const detail = msg.type === "SourceFailed" ? ` : ${msg.data.error} (nouvel essai dans ${msg.data.retry_in_ms} ms)` : "";
    console.log(`  ⚡ événement ${msg.app}.${msg.type}${detail}`);
  } else if (msg.op === "response") {
    const p = pending.get(msg.id);
    if (!p) return console.error("réponse sans requête :", msg);
    pending.delete(msg.id);
    msg.ok ? p.resolve(msg.data) : p.reject(new Error(`${p.type}: ${msg.error.code} (${msg.error.message})`));
  }
};

async function run() {
  console.log("requêtes :", (await request("GetRequestList")).requests.length);

  const enc = await request("GetEncoders");
  console.log(`encodeurs (backend ${enc.backend}, demandé : ${enc.requested}) :`);
  for (const e of enc.encoders) {
    const tags = [e.hardware ? "matériel" : "logiciel", e.available ? "disponible" : "absent"];
    if (e.gpl) tags.push("GPL");
    console.log(`  ${e.selected ? "→" : " "} ${e.name} (${tags.join(", ")})`);
  }

  const devices = await request("GetInputDevices");
  console.log(`périphériques : ${devices.video.length} vidéo, ${devices.audio.length} audio`);
  for (const d of [...devices.video, ...devices.audio]) {
    console.log(`  ${d.name} [${d.id}] ${d.caps_summary}`);
  }

  await request("CreateScene", { name: "Plateau" });
  await request("AddSource", { scene: "Plateau", name: "Fond", kind: "color", rgba: [20, 30, 60, 255] });
  await request("AddSource", {
    scene: "Plateau", name: "Incrustation", kind: "test_pattern",
    transform: { x: 760, y: 40, width: 480, height: 270 },
  });
  if (mediaPath) {
    await request("AddSource", {
      scene: "Plateau", name: "Clip", kind: "media_file", path: mediaPath, loop: true,
      transform: { x: 40, y: 40, width: 640, height: 360 },
    });
    await request("SetSourceVolume", { scene: "Plateau", source: "Clip", volume: 0.8 });
    console.log(`fichier média : ${mediaPath}`);
  }
  await request("AddSource", {
    scene: "Plateau", name: "Bandeau", kind: "color", rgba: [200, 30, 40, 255],
    transform: { x: 0, y: 600, width: 1280, height: 80, opacity: 0.85 },
  });

  const rec = await request("StartRecording", { format });
  console.log(`enregistrement vers ${rec.target} (${rec.format}, ${rec.encoder}, ${rec.bitrate_kbps} kbit/s)`);
  if (streamUrl) {
    const st = await request("StartStream", { url: streamUrl, bitrate_kbps: 2500 });
    console.log(`diffusion vers ${st.target} (${st.encoder})`);
  }
  await sleep(1500);

  await request("SetPreviewScene", { scene: "Plateau" });
  await request("Cut");
  await sleep(1500);

  await request("UpdateSource", { scene: "Plateau", source: "Bandeau", visible: false });
  if (mediaPath) {
    await request("SetSourceMuted", { scene: "Plateau", source: "Clip", muted: true });
    await sleep(500);
    await request("SetSourceMuted", { scene: "Plateau", source: "Clip", muted: false });
    await sleep(500);
  } else {
    await sleep(1000);
  }

  const status = await request("GetOutputStatus");
  console.log(`statut : ${status.record.frames} images, ${status.record.dropped_frames} perdues`);

  if (streamUrl) {
    const st = await request("StopStream");
    console.log(`diffusé : ${st.frames} images, ${st.dropped_frames} perdues`);
  }
  const stop = await request("StopRecording");
  console.log(
    `enregistré : ${stop.frames} images en ${stop.duration_s.toFixed(1)} s, ` +
    `${stop.dropped_frames} perdues, finalisé : ${stop.finalized}`,
  );

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
