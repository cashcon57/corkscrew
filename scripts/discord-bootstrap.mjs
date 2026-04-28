#!/usr/bin/env node
// Idempotent Discord server setup for the official Corkscrew server.
// Run: BOT_TOKEN=xxx GUILD_ID=yyy node scripts/discord-bootstrap.mjs
// Add --dry-run to preview without mutating.
//
// Bot needs scope=bot + Administrator permission.
// Invite URL: https://discord.com/api/oauth2/authorize?client_id=<APP_ID>&permissions=8&scope=bot

import { execFileSync } from "node:child_process";

const API = "https://discord.com/api/v10";
const TOKEN = process.env.BOT_TOKEN;
const GUILD = process.env.GUILD_ID;
const REPO = process.env.GITHUB_REPO || "cashcon57/corkscrew";
const DRY = process.argv.includes("--dry-run");
const PRUNE = process.argv.includes("--prune");
const RESEED = process.argv.includes("--reseed");

if (!TOKEN || !GUILD) {
  console.error("Missing BOT_TOKEN or GUILD_ID env var.");
  process.exit(1);
}

// ─── Permission bitfield helpers ──────────────────────────────────────────────
const P = {
  VIEW_CHANNEL:        1n << 10n,
  SEND_MESSAGES:       1n << 11n,
  MANAGE_MESSAGES:     1n << 13n,
  MENTION_EVERYONE:    1n << 17n,
  MANAGE_CHANNELS:     1n << 4n,
  MANAGE_ROLES:        1n << 28n,
  KICK_MEMBERS:        1n << 1n,
  BAN_MEMBERS:         1n << 2n,
  ADMINISTRATOR:       1n << 3n,
  MODERATE_MEMBERS:    1n << 40n,
  ADD_REACTIONS:       1n << 6n,
  ATTACH_FILES:        1n << 15n,
  EMBED_LINKS:         1n << 14n,
  READ_MESSAGE_HISTORY:1n << 16n,
};
const bits = (...flags) => flags.reduce((a, f) => a | f, 0n).toString();

// ─── Channel types ────────────────────────────────────────────────────────────
const CT = { TEXT: 0, VOICE: 2, CATEGORY: 4, ANNOUNCEMENT: 5 };

// ─── Roles ────────────────────────────────────────────────────────────────────
// Order = display order (top to bottom in this list = top to bottom in sidebar).
// hoist = shows as separate sidebar group.
const ROLES = [
  // Staff / status (hoisted)
  { name: "Moderator",       color: 0xED4245, hoist: true,  mentionable: true,
    permissions: bits(P.MANAGE_MESSAGES, P.KICK_MEMBERS, P.MODERATE_MEMBERS, P.MANAGE_ROLES) },
  { name: "Contributor",     color: 0xA855F7, hoist: true,  mentionable: true,  permissions: "0" },
  { name: "Mod Creator",     color: 0xF59E0B, hoist: true,  mentionable: true,  permissions: "0" },
  { name: "Tester",          color: 0x06B6D4, hoist: true,  mentionable: true,  permissions: "0" },
  { name: "Mod Wizard",      color: 0x8B5CF6, hoist: true,  mentionable: true,  permissions: "0" },

  // Adult opt-in (gates NSFW category)
  { name: "NSFW (18+)",      color: 0x991B1B, hoist: false, mentionable: false, permissions: "0" },

  // Platform self-assign
  { name: "macOS",           color: 0xE5E7EB, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Linux",           color: 0xFCC624, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Steam Deck",      color: 0x1A9FFF, hoist: false, mentionable: true,  permissions: "0" },

  // Runtime self-assign
  { name: "CrossOver",       color: 0xC850C0, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Whisky",          color: 0x6366F1, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Moonshine",       color: 0xBF5AF2, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Proton",          color: 0x1A9FFF, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Wine / Lutris",   color: 0xEC4899, hoist: false, mentionable: true,  permissions: "0" },

  // Game ping self-assign (consolidated to keep role list manageable)
  { name: "Skyrim SE",       color: 0x10B981, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Fallout 4",       color: 0x10B981, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Hades II",        color: 0xEF4444, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Sims 4",          color: 0x14B8A6, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Rockstar Games",  color: 0xF97316, hoist: false, mentionable: true,  permissions: "0" }, // GTA V, RDR2
  { name: "HoYo Games",      color: 0xFBBF24, hoist: false, mentionable: true,  permissions: "0" }, // Genshin, HSR, ZZZ, HI3
  { name: "BepInEx Games",   color: 0x22D3EE, hoist: false, mentionable: true,  permissions: "0" }, // Silksong, RoR2, Lethal, Palworld, Valheim
  { name: "Wabbajack",       color: 0x84CC16, hoist: false, mentionable: true,  permissions: "0" },

  // Experience + announce
  { name: "New Modder",      color: 0x9CA3AF, hoist: false, mentionable: true,  permissions: "0" },
  { name: "Releases",        color: 0x5865F2, hoist: false, mentionable: true,  permissions: "0" },
];

// ─── Channel tree (consolidated to ~21 channels for low-volume start) ─────────
const TREE = [
  { name: "📌 INFO", type: CT.CATEGORY, children: [
    { name: "welcome",            type: CT.TEXT, lockedToStaff: true },
    { name: "rules",              type: CT.TEXT, lockedToStaff: true },
    { name: "announcements",      type: CT.ANNOUNCEMENT, lockedToStaff: true },
    { name: "roles",              type: CT.TEXT, lockedToStaff: true },
  ]},

  { name: "💬 COMMUNITY", type: CT.CATEGORY, children: [
    { name: "general",            type: CT.TEXT, topic: "General Corkscrew chat." },
    { name: "showcase",           type: CT.TEXT, topic: "Modlists, screenshots, video — show what you built." },
    { name: "off-topic",          type: CT.TEXT, topic: "Memes and random chat." },
  ]},

  { name: "🛠 HELP", type: CT.CATEGORY, children: [
    { name: "mac-help",           type: CT.TEXT, topic: "macOS: CrossOver, Whisky, Moonshine, Mythic, native Wine." },
    { name: "linux-help",         type: CT.TEXT, topic: "Linux: Proton, Wine, Lutris, Bottles, Heroic, UMU, Steam Deck, CachyOS, Bazzite." },
    { name: "wabbajack-modlists", type: CT.TEXT, topic: "Wabbajack install pipeline + big lists like GTS, Lorerim." },
    { name: "compatibility-reports", type: CT.TEXT, topic: "What works / what breaks per game per runtime. Anti-cheat status here too." },
  ]},

  { name: "🎮 GAMES", type: CT.CATEGORY, children: [
    { name: "bethesda-modding",   type: CT.TEXT, topic: "Skyrim SE + Fallout 4: SKSE/F4SE, ESP load order, BSA/BA2." },
    { name: "hoyo-mods",          type: CT.TEXT, topic: "Genshin, HSR, ZZZ, HI3 — 3DMigoto / GIMI / SRMI / ZZMI." },
    { name: "bepinex-games",      type: CT.TEXT, topic: "Unity + BepInEx: Silksong, RoR2, Lethal, Palworld, Valheim, etc." },
    { name: "other-games",        type: CT.TEXT, topic: "Sims 4, Hades II, Rockstar, Cyberpunk, Witcher 3, anything else." },
  ]},

  { name: "🔧 DEV", type: CT.CATEGORY, children: [
    { name: "dev-chat",           type: CT.TEXT, topic: "Architecture, design, contributors, bug triage, feature ideas." },
    { name: "nightly-builds",     type: CT.TEXT, topic: "Bleeding-edge builds, Tester role only.", testerOnly: true },
    { name: "releases",           type: CT.TEXT, topic: "GitHub release webhook target.", webhook: "GitHub Releases" },
    { name: "github",             type: CT.TEXT, topic: "Issues + PR webhook target.",   webhook: "GitHub Activity" },
  ]},

  { name: "🔊 VOICE", type: CT.CATEGORY, children: [
    { name: "general-voice",      type: CT.VOICE },
  ]},

  { name: "🔞 ADULT (18+)", type: CT.CATEGORY, nsfwGated: true, children: [
    { name: "nsfw-mods",          type: CT.TEXT, nsfw: true, nsfwGated: true, topic: "Adult mods (LoversLab, etc). Requires NSFW (18+) opt-in role." },
  ]},

  { name: "🔒 STAFF", type: CT.CATEGORY, staffOnly: true, children: [
    { name: "mod-chat",           type: CT.TEXT, staffOnly: true },
    { name: "mod-actions",        type: CT.TEXT, staffOnly: true, webhook: "Audit Log" },
    { name: "message-log",        type: CT.TEXT, staffOnly: true, topic: "Carl-bot logs message edits and deletes here." },
    { name: "moderator-only",     type: CT.TEXT, staffOnly: true }, // Discord-required community safety channel
  ]},
];

// ─── Seed messages ────────────────────────────────────────────────────────────
const RULES_BODY = `**Corkscrew Discord — Rules**

**1. Be decent.** No harassment, slurs, doxxing, or bigotry. We follow the project Code of Conduct: <https://github.com/cashcon57/corkscrew/blob/main/CODE_OF_CONDUCT.md>

**2. Don't pirate the base game.** Own your games legitimately (Steam, GOG, etc). The base game files aren't ours to share.

**3. Modding is free.** Mods, modlists, paid mods, leaked Patreon content — share away. That's the culture. We don't gatekeep mod access.

**4. Use the right channel.**
• On macOS? → <#mac-help>
• On Linux / Steam Deck? → <#linux-help>
• Wabbajack modlist? → <#wabbajack-modlists>
• Game-specific (Bethesda / HoYo / Unity / etc)? → the matching channel under 🎮 GAMES
• Bug or feature idea? → <#dev-chat>
• Just chatting? → <#general> / <#off-topic>

**5. Search before asking.** Use Discord search + check <#announcements> for known issues.

**6. Help requests need info.** Include: app version, OS, Wine/Proton source + version, game, what you tried. "It doesn't work" gets ignored.

**7. NSFW is opt-in.** The 🔞 category is hidden by default. Grab the **NSFW (18+)** role to access it. Posting adult content outside that category = instant timeout.

**8. No advertising.** Don't shill your Discord, Patreon, or paid service unless asked. Modders linking their own mods = fine.

**9. Bug reports go to GitHub.** Discord triages. Confirmed bugs → <https://github.com/cashcon57/corkscrew/issues>

**10. Staff have final say.** Disagree? DM a Moderator. Don't argue in-channel.

Breaking rules = warn → timeout → kick → ban.`;

// Reaction roles. emoji → role name. Carl-bot will be configured to listen to these.
// Discord caps at 20 reactions per message — this list is at exactly 20.
const ROLE_REACTIONS = [
  ["🍎", "macOS"],
  ["🐧", "Linux"],
  ["🎮", "Steam Deck"],
  ["🍷", "CrossOver"],
  ["🥃", "Whisky"],
  ["🌙", "Moonshine"],
  ["🚀", "Proton"],
  ["🍇", "Wine / Lutris"],
  ["⚔️", "Skyrim SE"],
  ["☢️", "Fallout 4"],
  ["📦", "Wabbajack"],
  ["🔥", "Hades II"],
  ["🏠", "Sims 4"],
  ["🤠", "Rockstar Games"],
  ["✨", "HoYo Games"],
  ["🎲", "BepInEx Games"],
  ["📢", "Releases"],
  ["🌱", "New Modder"],
  ["🧙", "Mod Wizard"],
  ["🔞", "NSFW (18+)"],
];

const ROLES_BODY = `**🎭 Pick your roles**

Click an emoji below to get a role. Click again to remove.

**Platform**
🍎 macOS · 🐧 Linux · 🎮 Steam Deck

**Wine / Proton runtime**
🍷 CrossOver · 🥃 Whisky · 🌙 Moonshine · 🚀 Proton · 🍇 Wine / Lutris

**Games you mod**
⚔️ Skyrim SE · ☢️ Fallout 4 · 📦 Wabbajack · 🔥 Hades II · 🏠 Sims 4
🤠 GTA V / RDR2 · ✨ Genshin / HSR / ZZZ · 🎲 Unity / BepInEx

**Notifications & experience**
📢 Release pings · 🌱 New Modder · 🧙 Mod Wizard

**Adult content**
🔞 NSFW (18+) — unlocks the 🔞 ADULT category`;

const WELCOME_BODY = `👋 **Welcome to the Corkscrew Discord.**

Corkscrew is a modern mod manager for running Windows games on **macOS and Linux**. CrossOver, Whisky, Moonshine, Mythic, Proton, Wine, Lutris, Bottles, Heroic, UMU — if you can run the game, you can mod it.

🌐 **Website:**  https://corkscrewmodmanager.com
💾 **Download:** https://github.com/cashcon57/corkscrew/releases
📖 **README:**   https://github.com/cashcon57/corkscrew#readme
🐛 **Bugs:**     https://github.com/cashcon57/corkscrew/issues
☕ **Ko-fi:**     https://ko-fi.com/cash508287

**Before you post:**
→ read <#rules>
→ grab your roles in **Channels & Roles** (top of channel list) — pick your platform, runtime, games you mod, and opt into NSFW if you want it
→ check <#announcements> for known issues

**Where to post:**
• On macOS? → <#mac-help>
• On Linux / Steam Deck? → <#linux-help>
• Wabbajack modlist? → <#wabbajack-modlists>
• Bethesda (Skyrim / Fallout)? → <#bethesda-modding>
• Genshin / HSR / ZZZ? → <#hoyo-mods>
• Unity / BepInEx (Silksong, RoR2, Lethal, Palworld, etc)? → <#bepinex-games>
• Anything else (Sims, Hades II, GTA, Cyberpunk, etc)? → <#other-games>
• Bug or feature idea? → <#dev-chat> (or file on GitHub)

Goal: make modding Windows games on Mac and Linux **at least as good as Windows — often better.** 🍷`;

// ─── HTTP helper with rate-limit retry ────────────────────────────────────────
async function api(method, path, body) {
  if (DRY && method !== "GET") {
    console.log(`  [dry] ${method} ${path}`);
    return null;
  }
  for (let attempt = 0; attempt < 5; attempt++) {
    const res = await fetch(`${API}${path}`, {
      method,
      headers: {
        Authorization: `Bot ${TOKEN}`,
        "Content-Type": "application/json",
        "User-Agent": "CorkscrewBootstrap (https://github.com/cashcon57/corkscrew, 1.0)",
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (res.status === 429) {
      const retry = parseFloat(res.headers.get("retry-after") || "1");
      console.log(`  rate-limited, sleeping ${retry}s`);
      await new Promise(r => setTimeout(r, retry * 1000));
      continue;
    }
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`${method} ${path} → ${res.status}: ${text}`);
    }
    if (res.status === 204) return null;
    return res.json();
  }
  throw new Error(`Gave up after rate-limit retries: ${method} ${path}`);
}

// ─── Sync roles ───────────────────────────────────────────────────────────────
async function syncRoles() {
  console.log("\n▶ Roles");
  const existing = await api("GET", `/guilds/${GUILD}/roles`);
  const byName = new Map(existing.map(r => [r.name, r]));
  const result = new Map(existing.map(r => [r.name, r]));

  for (const def of ROLES) {
    const cur = byName.get(def.name);
    if (cur) {
      const drift =
        cur.color !== def.color ||
        cur.hoist !== def.hoist ||
        cur.mentionable !== def.mentionable ||
        cur.permissions !== def.permissions;
      if (drift) {
        console.log(`  ↻ patch role: ${def.name}`);
        const updated = await api("PATCH", `/guilds/${GUILD}/roles/${cur.id}`, def);
        if (updated) result.set(def.name, updated);
      } else {
        console.log(`  ✓ role exists: ${def.name}`);
      }
    } else {
      console.log(`  + create role: ${def.name}`);
      const created = await api("POST", `/guilds/${GUILD}/roles`, def);
      if (created) result.set(def.name, created);
    }
  }
  return result;
}

// ─── Permission overwrite generators ──────────────────────────────────────────
function staffOnlyOverwrites(roles) {
  const everyoneId = GUILD;
  const mod = roles.get("Moderator");
  const out = [{ id: everyoneId, type: 0, allow: "0", deny: bits(P.VIEW_CHANNEL) }];
  if (mod) out.push({ id: mod.id, type: 0, allow: bits(P.VIEW_CHANNEL, P.SEND_MESSAGES, P.MANAGE_MESSAGES), deny: "0" });
  return out;
}

function readOnlyOverwrites(roles) {
  const everyoneId = GUILD;
  const mod = roles.get("Moderator");
  const out = [{
    id: everyoneId, type: 0,
    allow: bits(P.VIEW_CHANNEL, P.READ_MESSAGE_HISTORY, P.ADD_REACTIONS),
    deny:  bits(P.SEND_MESSAGES),
  }];
  if (mod) out.push({ id: mod.id, type: 0, allow: bits(P.SEND_MESSAGES, P.MANAGE_MESSAGES), deny: "0" });
  return out;
}

function nsfwGatedOverwrites(roles) {
  const everyoneId = GUILD;
  const nsfw = roles.get("NSFW (18+)");
  const mod = roles.get("Moderator");
  const out = [{ id: everyoneId, type: 0, allow: "0", deny: bits(P.VIEW_CHANNEL) }];
  if (nsfw) out.push({ id: nsfw.id, type: 0, allow: bits(P.VIEW_CHANNEL, P.SEND_MESSAGES, P.READ_MESSAGE_HISTORY, P.ADD_REACTIONS, P.ATTACH_FILES, P.EMBED_LINKS), deny: "0" });
  if (mod)  out.push({ id: mod.id,  type: 0, allow: bits(P.VIEW_CHANNEL, P.SEND_MESSAGES, P.MANAGE_MESSAGES), deny: "0" });
  return out;
}

function testerOnlyOverwrites(roles) {
  const everyoneId = GUILD;
  const tester = roles.get("Tester");
  const mod = roles.get("Moderator");
  const out = [{ id: everyoneId, type: 0, allow: "0", deny: bits(P.VIEW_CHANNEL) }];
  if (tester) out.push({ id: tester.id, type: 0, allow: bits(P.VIEW_CHANNEL, P.SEND_MESSAGES, P.READ_MESSAGE_HISTORY, P.ADD_REACTIONS, P.ATTACH_FILES, P.EMBED_LINKS), deny: "0" });
  if (mod)    out.push({ id: mod.id,    type: 0, allow: bits(P.VIEW_CHANNEL, P.SEND_MESSAGES, P.MANAGE_MESSAGES), deny: "0" });
  return out;
}

function normalizeOverwrites(arr) {
  return [...arr].map(o => ({
    id: String(o.id),
    type: Number(o.type),
    allow: String(o.allow ?? "0"),
    deny: String(o.deny ?? "0"),
  })).sort((a, b) => a.id.localeCompare(b.id));
}

function overwritesFor(node, roles) {
  if (node.staffOnly)   return staffOnlyOverwrites(roles);
  if (node.testerOnly)  return testerOnlyOverwrites(roles);
  if (node.nsfwGated)   return nsfwGatedOverwrites(roles);
  if (node.lockedToStaff) return readOnlyOverwrites(roles);
  return undefined;
}

// ─── Sync channels ────────────────────────────────────────────────────────────
async function syncChannels(roles) {
  console.log("\n▶ Channels");
  const existing = await api("GET", `/guilds/${GUILD}/channels`);
  const byName = new Map(existing.map(c => [c.name.toLowerCase(), c]));

  // Categories first (need their IDs for parent_id on children).
  const categoryIds = new Map();
  for (const cat of TREE) {
    const key = cat.name.toLowerCase();
    let chan = byName.get(key);
    const overwrites = overwritesFor(cat, roles);
    if (chan) {
      console.log(`  ✓ category exists: ${cat.name}`);
    } else {
      console.log(`  + create category: ${cat.name}`);
      chan = await api("POST", `/guilds/${GUILD}/channels`, {
        name: cat.name,
        type: CT.CATEGORY,
        permission_overwrites: overwrites,
      });
    }
    if (chan) categoryIds.set(cat.name, chan.id);
  }

  // Children. Match by name+type so a voice "General" doesn't collide with text "general".
  const childRefs = new Map();
  for (const cat of TREE) {
    const parentId = categoryIds.get(cat.name);
    for (const ch of cat.children) {
      let chan = existing.find(c => c.name.toLowerCase() === ch.name.toLowerCase() && c.type === ch.type);
      const overwrites = overwritesFor(ch, roles);
      const payload = {
        name: ch.name,
        type: ch.type,
        parent_id: parentId,
        topic: ch.topic,
        nsfw: ch.nsfw === true,
        permission_overwrites: overwrites,
      };
      try {
        if (chan) {
          const wantOverwrites = overwrites || [];
          const haveOverwrites = chan.permission_overwrites || [];
          const overwritesDrift = JSON.stringify(normalizeOverwrites(wantOverwrites)) !==
                                  JSON.stringify(normalizeOverwrites(haveOverwrites));
          const drift =
            chan.parent_id !== parentId ||
            chan.type !== ch.type ||
            (ch.topic && chan.topic !== ch.topic) ||
            (!!chan.nsfw) !== (!!ch.nsfw) ||
            (overwrites && overwritesDrift);
          if (drift) {
            console.log(`  ↻ patch channel: ${ch.name}${overwrites && overwritesDrift ? " (perms)" : ""}`);
            const patch = {
              parent_id: parentId,
              topic: ch.topic,
              nsfw: !!ch.nsfw,
            };
            if (overwrites) patch.permission_overwrites = overwrites;
            try {
              chan = await api("PATCH", `/channels/${chan.id}`, patch) || chan;
            } catch (e) {
              if (e.message.includes("CHANNEL_TOPIC_INVALID")) {
                console.log(`    ⚠ topic blocked by Discord filter, retrying without topic`);
                const { topic: _, ...patchNoTopic } = patch;
                chan = await api("PATCH", `/channels/${chan.id}`, patchNoTopic) || chan;
              } else { throw e; }
            }
          } else {
            console.log(`  ✓ channel exists: ${ch.name}`);
          }
        } else {
          console.log(`  + create channel: ${ch.name}`);
          try {
            chan = await api("POST", `/guilds/${GUILD}/channels`, payload);
          } catch (e) {
            if (e.message.includes("CHANNEL_TOPIC_INVALID")) {
              console.log(`    ⚠ topic blocked by Discord filter, retrying without topic`);
              chan = await api("POST", `/guilds/${GUILD}/channels`, { ...payload, topic: undefined });
            } else { throw e; }
          }
        }
        if (chan) childRefs.set(ch.name, { ...chan, def: ch });
      } catch (e) {
        console.log(`  ✗ failed channel: ${ch.name} — ${e.message}`);
      }
    }
  }
  return childRefs;
}

// ─── Post welcome/rules if channel is empty (or --reseed forces overwrite) ────
async function postSeedMessages(channels) {
  console.log("\n▶ Seed messages" + (RESEED ? " (--reseed: rewriting non-empty)" : ""));
  const seeds = [
    { channel: "welcome", body: WELCOME_BODY },
    { channel: "rules",   body: RULES_BODY },
  ];
  for (const { channel, body } of seeds) {
    const ch = channels.get(channel);
    if (!ch) { console.log(`  ✗ channel not found: ${channel}`); continue; }
    const msgs = await api("GET", `/channels/${ch.id}/messages?limit=10`);
    const hasMsgs = msgs && msgs.length > 0;
    if (hasMsgs && !RESEED) {
      console.log(`  ✓ ${channel} already has messages, skipping`);
      continue;
    }
    if (hasMsgs && RESEED) {
      console.log(`  ↻ deleting ${msgs.length} existing message(s) in ${channel}`);
      for (const m of msgs) {
        try { await api("DELETE", `/channels/${ch.id}/messages/${m.id}`); }
        catch (e) { console.log(`    ✗ could not delete ${m.id}: ${e.message.split("\n")[0]}`); }
      }
    }
    console.log(`  + posting seed in ${channel}`);
    await api("POST", `/channels/${ch.id}/messages`, { content: body });
  }
}

// ─── Roles message + reactions (for Carl-bot reaction roles) ─────────────────
async function postRolesMessage(channels) {
  console.log("\n▶ Roles message + reactions");
  const ch = channels.get("roles");
  if (!ch) { console.log("  ✗ #roles channel not found"); return null; }
  const existing = await api("GET", `/channels/${ch.id}/messages?limit=10`);
  let msg = (existing || []).find(m => m.content?.includes("Pick your roles"));
  if (msg && !RESEED) {
    console.log(`  ✓ roles message exists (id ${msg.id})`);
  } else {
    if (msg && RESEED) {
      console.log(`  ↻ deleting old roles message`);
      try { await api("DELETE", `/channels/${ch.id}/messages/${msg.id}`); } catch {}
    }
    console.log(`  + posting roles message`);
    if (DRY) { console.log("  [dry] POST message + reactions"); return null; }
    msg = await api("POST", `/channels/${ch.id}/messages`, { content: ROLES_BODY });
  }
  if (!msg) return null;

  // Add reactions (idempotent — Discord ignores duplicates).
  console.log(`  + adding ${ROLE_REACTIONS.length} reactions`);
  for (const [emoji] of ROLE_REACTIONS) {
    if (DRY) { console.log(`    [dry] react ${emoji}`); continue; }
    const enc = encodeURIComponent(emoji);
    try { await api("PUT", `/channels/${ch.id}/messages/${msg.id}/reactions/${enc}/@me`); }
    catch (e) { console.log(`    ✗ react ${emoji}: ${e.message.split("\n")[0]}`); }
  }
  console.log(`  ↪ message id: ${msg.id}  (give this to Carl-bot reaction-role config)`);
  return msg.id;
}

// ─── Prune: delete channels and categories not in TREE ────────────────────────
async function pruneOrphans() {
  if (!PRUNE) return;
  console.log("\n▶ Prune (delete channels/categories not in TREE)");
  const all = await api("GET", `/guilds/${GUILD}/channels`);
  const wantedCategories = new Set(TREE.map(c => c.name.toLowerCase()));
  // Match name+type so a default voice "General" doesn't shield a text "general".
  const wantedChildren = new Set(TREE.flatMap(c => c.children.map(ch => `${ch.name.toLowerCase()}|${ch.type}`)));

  // Delete children first (so categories can be deleted after).
  for (const ch of all) {
    if (ch.type === CT.CATEGORY) continue;
    const key = `${ch.name.toLowerCase()}|${ch.type}`;
    if (wantedChildren.has(key)) continue;
    console.log(`  - delete channel: ${ch.name} (type ${ch.type})`);
    if (DRY) { console.log(`    [dry] DELETE /channels/${ch.id}`); continue; }
    try { await api("DELETE", `/channels/${ch.id}`); }
    catch (e) { console.log(`    ✗ failed: ${e.message.split("\n")[0]}`); }
  }
  // Delete empty/unwanted categories.
  for (const ch of all) {
    if (ch.type !== CT.CATEGORY) continue;
    const name = ch.name.toLowerCase();
    if (wantedCategories.has(name)) continue;
    console.log(`  - delete category: ${ch.name}`);
    if (DRY) { console.log(`    [dry] DELETE /channels/${ch.id}`); continue; }
    try { await api("DELETE", `/channels/${ch.id}`); }
    catch (e) { console.log(`    ✗ failed: ${e.message.split("\n")[0]}`); }
  }
}

// ─── Webhooks ─────────────────────────────────────────────────────────────────
async function syncWebhooks(channels) {
  console.log("\n▶ Webhooks");
  const out = {};
  for (const [name, ch] of channels.entries()) {
    if (!ch.def?.webhook) continue;
    const existing = await api("GET", `/channels/${ch.id}/webhooks`);
    let hook = existing.find(w => w.name === ch.def.webhook);
    if (hook) {
      console.log(`  ✓ webhook exists: ${ch.def.webhook} (#${name})`);
    } else {
      console.log(`  + create webhook: ${ch.def.webhook} (#${name})`);
      hook = await api("POST", `/channels/${ch.id}/webhooks`, { name: ch.def.webhook });
    }
    if (hook?.token) {
      out[`${name}/${ch.def.webhook}`] = `https://discord.com/api/webhooks/${hook.id}/${hook.token}`;
    }
  }
  return out;
}

// ─── AutoMod ──────────────────────────────────────────────────────────────────
async function syncAutomod() {
  console.log("\n▶ AutoMod");
  const existing = await api("GET", `/guilds/${GUILD}/auto-moderation/rules`);
  const byName = new Map(existing.map(r => [r.name, r]));
  const rules = [
    {
      name: "Block Discord-preset slurs",
      event_type: 1,
      trigger_type: 4,
      trigger_metadata: { presets: [1, 2, 3] }, // PROFANITY, SEXUAL_CONTENT, SLURS
      actions: [{ type: 1, metadata: { custom_message: "Message blocked by AutoMod (preset filter)." } }],
      enabled: true,
    },
    {
      name: "Block mass mentions",
      event_type: 1,
      trigger_type: 5,
      trigger_metadata: { mention_total_limit: 5 },
      actions: [{ type: 1, metadata: { custom_message: "Too many mentions in one message." } }],
      enabled: true,
    },
    {
      name: "Block invite spam",
      event_type: 1,
      trigger_type: 1, // KEYWORD
      trigger_metadata: {
        keyword_filter: ["discord.gg/*", "discord.com/invite/*", "discordapp.com/invite/*"],
      },
      actions: [{ type: 1, metadata: { custom_message: "Discord invites are not allowed. Ask a Moderator." } }],
      enabled: true,
      exempt_roles: [],
    },
  ];
  for (const r of rules) {
    if (byName.has(r.name)) {
      console.log(`  ✓ automod rule exists: ${r.name}`);
    } else {
      console.log(`  + create automod rule: ${r.name}`);
      try { await api("POST", `/guilds/${GUILD}/auto-moderation/rules`, r); }
      catch (e) { console.log(`  ✗ failed (${r.name}): ${e.message}`); }
    }
  }
}

// ─── Community + verification + rules screening ───────────────────────────────
async function syncCommunityFields(channels) {
  console.log("\n▶ Community fields (rules + updates channels, verification)");
  const guild = await api("GET", `/guilds/${GUILD}`);
  const rulesId = channels.get("rules")?.id;
  const updatesId = channels.get("announcements")?.id;
  const patch = {};
  if (rulesId && guild.rules_channel_id !== rulesId) patch.rules_channel_id = rulesId;
  if (updatesId && guild.public_updates_channel_id !== updatesId) patch.public_updates_channel_id = updatesId;
  if (guild.verification_level < 2) patch.verification_level = 2;
  if (Object.keys(patch).length === 0) {
    console.log("  ✓ already configured");
    return;
  }
  console.log(`  ↻ patch guild: ${Object.keys(patch).join(", ")}`);
  try { await api("PATCH", `/guilds/${GUILD}`, patch); }
  catch (e) { console.log(`  ✗ failed: ${e.message}`); }
}

// ─── Onboarding ───────────────────────────────────────────────────────────────
async function syncOnboarding(roles, channels) {
  console.log("\n▶ Onboarding");
  let cur;
  try { cur = await api("GET", `/guilds/${GUILD}/onboarding`); } catch (e) { cur = null; }
  if (cur && cur.enabled && (cur.prompts?.length ?? 0) > 0 && !PRUNE && !RESEED) {
    console.log(`  ✓ already configured (${cur.prompts.length} prompts) — leaving alone (pass --prune or --reseed to force re-sync)`);
    return;
  }

  const r = name => roles.get(name)?.id;
  const c = name => channels.get(name)?.id;
  const filter = arr => arr.filter(Boolean);

  const defaults = filter(["welcome","rules","announcements","general","showcase","off-topic","mac-help","linux-help","wabbajack-modlists"].map(c));
  if (defaults.length < 7) {
    console.log(`  ✗ need ≥7 default channels visible to @everyone (have ${defaults.length}), skipping`);
    return;
  }

  const prompts = [
    {
      id: "0", type: 0, single_select: true, required: true, in_onboarding: true,
      title: "What platform are you on?",
      options: filter([
        r("macOS")      && { id: "0", title: "macOS",      role_ids: [r("macOS")],      channel_ids: [], emoji: { name: "🍎" } },
        r("Linux")      && { id: "1", title: "Linux",      role_ids: [r("Linux")],      channel_ids: [], emoji: { name: "🐧" } },
        r("Steam Deck") && { id: "2", title: "Steam Deck", role_ids: [r("Steam Deck")], channel_ids: [], emoji: { name: "🎮" } },
      ]),
    },
    {
      id: "1", type: 0, single_select: false, required: false, in_onboarding: true,
      title: "Which Wine / Proton runtime do you use?",
      options: filter([
        r("CrossOver")     && { id: "0", title: "CrossOver",     role_ids: [r("CrossOver")],     channel_ids: filter([c("crossover-help")]),  emoji: { name: "🍷" } },
        r("Whisky")        && { id: "1", title: "Whisky",        role_ids: [r("Whisky")],        channel_ids: filter([c("whisky-help")]),     emoji: { name: "🥃" } },
        r("Moonshine")     && { id: "2", title: "Moonshine",     role_ids: [r("Moonshine")],     channel_ids: filter([c("moonshine-mythic")]),emoji: { name: "🌙" } },
        r("Proton")        && { id: "3", title: "Proton",        role_ids: [r("Proton")],        channel_ids: filter([c("proton-help")]),     emoji: { name: "🚀" } },
        r("Wine / Lutris") && { id: "4", title: "Wine / Lutris", role_ids: [r("Wine / Lutris")], channel_ids: filter([c("wine-lutris-help")]),emoji: { name: "🍇" } },
      ]),
    },
    {
      id: "2", type: 0, single_select: false, required: false, in_onboarding: true,
      title: "Which games do you mod?",
      options: filter([
        r("Skyrim SE")     && { id: "0", title: "Skyrim SE",        role_ids: [r("Skyrim SE")],     channel_ids: filter([c("skyrim-help")]),         emoji: { name: "⚔️" } },
        r("Fallout 4")     && { id: "1", title: "Fallout 4",        role_ids: [r("Fallout 4")],     channel_ids: filter([c("fallout-help")]),        emoji: { name: "☢️" } },
        r("Wabbajack")     && { id: "2", title: "Wabbajack lists",  role_ids: [r("Wabbajack")],     channel_ids: filter([c("wabbajack-modlists")]),  emoji: { name: "📦" } },
        r("Hades II")      && { id: "3", title: "Hades II",         role_ids: [r("Hades II")],      channel_ids: filter([c("hades2-help")]),         emoji: { name: "🔥" } },
        r("Sims 4")        && { id: "4", title: "The Sims 4",       role_ids: [r("Sims 4")],        channel_ids: filter([c("sims4-help")]),          emoji: { name: "🏠" } },
        r("Rockstar Games")&& { id: "5", title: "GTA V / RDR2",     role_ids: [r("Rockstar Games")],channel_ids: filter([c("rockstar-help")]),       emoji: { name: "🤠" } },
        r("HoYo Games")    && { id: "6", title: "Genshin / HSR / ZZZ", role_ids: [r("HoYo Games")], channel_ids: filter([c("hoyo-mods")]),           emoji: { name: "✨" } },
        r("BepInEx Games") && { id: "7", title: "Unity / BepInEx",  role_ids: [r("BepInEx Games")], channel_ids: filter([c("bepinex-games")]),       emoji: { name: "🎲" } },
      ]),
    },
    {
      id: "3", type: 0, single_select: false, required: false, in_onboarding: true,
      title: "Adult mod content (18+)?",
      options: filter([
        r("NSFW (18+)") && { id: "0", title: "Yes, I am 18+", role_ids: [r("NSFW (18+)")], channel_ids: filter([c("nsfw-mods"), c("loverslab-talk")]), emoji: { name: "🔞" } },
      ]),
    },
  ].filter(p => p.options.length > 0);

  console.log(`  + configuring onboarding (${prompts.length} prompts, ${defaults.length} default channels)`);
  if (DRY) { console.log("  [dry] PUT onboarding"); return; }
  try {
    await api("PUT", `/guilds/${GUILD}/onboarding`, {
      prompts,
      default_channel_ids: defaults,
      enabled: true,
      mode: 0,
    });
  } catch (e) {
    console.log(`  ✗ onboarding failed: ${e.message}`);
  }
}

// ─── Permanent invite ─────────────────────────────────────────────────────────
async function syncInvite(channels) {
  console.log("\n▶ Permanent invite");
  const welcome = channels.get("welcome");
  if (!welcome) { console.log("  ✗ no #welcome channel"); return null; }
  const existing = await api("GET", `/channels/${welcome.id}/invites`);
  const permanent = (existing || []).find(i => i.max_age === 0 && i.max_uses === 0);
  if (permanent) {
    const url = `https://discord.gg/${permanent.code}`;
    console.log(`  ✓ permanent invite exists: ${url}`);
    return url;
  }
  if (DRY) { console.log("  [dry] POST invite"); return null; }
  const inv = await api("POST", `/channels/${welcome.id}/invites`, {
    max_age: 0, max_uses: 0, unique: false, temporary: false,
  });
  if (inv?.code) {
    const url = `https://discord.gg/${inv.code}`;
    console.log(`  + created permanent invite: ${url}`);
    return url;
  }
  return null;
}

// ─── GitHub webhooks via gh CLI (execFile, no shell) ──────────────────────────
function gh(...args) {
  return execFileSync("gh", args, { encoding: "utf8" });
}

function wireGithubWebhooks(webhookUrls) {
  console.log("\n▶ GitHub webhooks (via gh CLI)");
  if (DRY) { console.log("  [dry] skipping"); return; }
  try { execFileSync("gh", ["auth", "status"], { stdio: "ignore" }); }
  catch { console.log("  ✗ gh CLI not authenticated. Run: gh auth login"); return; }

  const wantHooks = [
    { url: webhookUrls["releases/GitHub Releases"], events: ["release"], label: "releases" },
    { url: webhookUrls["github/GitHub Activity"],   events: ["issues","pull_request","issue_comment","discussion"], label: "github" },
  ].filter(h => h.url);

  let existing;
  try { existing = JSON.parse(gh("api", `repos/${REPO}/hooks`)); }
  catch (e) { console.log(`  ✗ couldn't list hooks: ${e.message.split("\n")[0]}`); return; }

  for (const h of wantHooks) {
    const url = `${h.url}/github`;
    if (existing.find(x => x.config?.url === url)) {
      console.log(`  ✓ webhook exists: ${h.label}`);
      continue;
    }
    const args = [
      "api", `repos/${REPO}/hooks`, "-X", "POST",
      "-f", "name=web", "-F", "active=true",
      "-f", "config[url]=" + url,
      "-f", "config[content_type]=json",
    ];
    for (const ev of h.events) args.push("-f", `events[]=${ev}`);
    try {
      execFileSync("gh", args, { stdio: "pipe" });
      console.log(`  + webhook created: ${h.label}`);
    } catch (e) {
      const msg = (e.stderr?.toString() || e.message).split("\n")[0];
      console.log(`  ✗ webhook ${h.label} failed: ${msg}`);
    }
  }
}

// ─── Main ─────────────────────────────────────────────────────────────────────
async function main() {
  const guild = await api("GET", `/guilds/${GUILD}`);
  console.log(`Connected to: ${guild.name} (${GUILD})`);
  if (DRY) console.log("DRY RUN — no mutations.\n");

  const roles = await syncRoles();
  const channels = await syncChannels(roles);
  await pruneOrphans();
  await postSeedMessages(channels);
  const rolesMsgId = await postRolesMessage(channels);
  const webhooks = await syncWebhooks(channels);
  await syncAutomod();
  await syncCommunityFields(channels);
  await syncOnboarding(roles, channels);
  const invite = await syncInvite(channels);
  wireGithubWebhooks(webhooks);

  console.log("\n✓ Done.");
  if (invite) console.log(`\n🔗 Public invite: ${invite}`);
  if (rolesMsgId) console.log(`🎭 Reaction-role message ID: ${rolesMsgId}`);
}

main().catch(err => { console.error(err); process.exit(1); });
