// Hermetic seed helper for the Omloppsbana visual gate.
//
// Talks ONLY to the local dev server's REST API (http://localhost:3000 by
// default) which, per `db.rs`, defaults to SurrealDB ns `authlyn` / db `dev`.
// NEVER point this at the prod URL or `SURREAL_DB=prod` / the novahome test
// deck — see the root CLAUDE.md prod-guardrail. The dev DB is disposable;
// every run registers a brand-new random user so reruns never collide.
//
// What it builds, matched to what the orbit swipe-strip needs to exercise:
//   - a fresh random account (auto-logged-in; we capture its session token);
//   - GUILD A — SINGLE channel: a new guild ships with exactly one default
//     `general` text channel (`persist_create_guild`), so we create it and
//     add nothing. Its neighbor peeks MUST both be "orbit's edge".
//   - GUILD B — MULTI channel: a new guild + two extra text channels = 3
//     total, so an interior channel has a real named neighbor either side.
//
// Pure Node (global fetch, Node 18+). No Playwright dependency here so the
// seed can be smoke-run on its own: `node seed.mjs`.

const BASE = process.env.AUTHLYN_GATE_URL || "http://localhost:3000";

// The server's session cookie name (`session.rs` SESSION_COOKIE). The cookie
// is HttpOnly + Secure, so the browser spec can't read it from JS — we parse
// the token out of login's Set-Cookie here and the runner injects it via
// context.addCookies (with secure:false for WebKit/localhost; see run-gate).
export const SESSION_COOKIE = "authlyn_session";

/** A registration-valid random username: alphanumeric, length 3..=32, no
 *  whitespace (`crypto.rs` validate_credentials). Prefixed so seeded rows are
 *  obvious if one ever leaks into a DB inspection. */
function randomUsername() {
  const rand = Math.random().toString(36).slice(2, 10);
  return `gatebot${rand}`;
}

/** POST JSON and return { status, headers, body }. Throws on network error
 *  only; callers decide what HTTP statuses are acceptable. */
async function postJson(path, body, cookie) {
  const headers = { "content-type": "application/json" };
  if (cookie) headers.cookie = cookie;
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
    redirect: "manual",
  });
  let json = null;
  try {
    json = await res.json();
  } catch {
    /* some endpoints (none used here) may not return JSON */
  }
  return { status: res.status, headers: res.headers, body: json };
}

/** Extract the `authlyn_session` token from a Set-Cookie header value. axum's
 *  cookie jar emits one Set-Cookie per cookie; Node's fetch coalesces multiple
 *  into a comma-joined string, but cookie *attributes* (Expires, Max-Age) also
 *  contain commas, so we anchor on the `<name>=` boundary rather than split. */
function tokenFromSetCookie(setCookie) {
  if (!setCookie) return null;
  const m = setCookie.match(new RegExp(`${SESSION_COOKIE}=([^;,\\s]+)`));
  return m ? m[1] : null;
}

/**
 * Register a fresh user and create the two guilds the gate asserts against.
 * @param {string} [base] override the dev server URL (defaults to env/localhost).
 * @returns {Promise<{username, password, token, single: {id,name}, multi: {id,name}}>}
 */
export async function seed(base = BASE) {
  if (/\bprod\b/i.test(base) || /\bdeck\b/i.test(base)) {
    throw new Error(
      `Refusing to seed against a prod/deck-looking URL: ${base}. ` +
        `The visual gate is dev-only (localhost:3000 + the dev DB).`,
    );
  }

  const username = randomUsername();
  const password = "gate-pw-12345"; // >= 8 chars (validate_password)

  // 1) Register → 201 with the session cookie in Set-Cookie.
  const reg = await postJson("/auth/register", { username, password });
  if (reg.status !== 201) {
    throw new Error(
      `register failed (${reg.status}): ${JSON.stringify(reg.body)}`,
    );
  }
  const token = tokenFromSetCookie(reg.headers.get("set-cookie"));
  if (!token) {
    throw new Error("register succeeded but no session cookie was returned");
  }
  const cookie = `${SESSION_COOKIE}=${token}`;

  // 2) GUILD A — single channel (ships with `general` only).
  const single = await postJson("/guilds", { name: "Edge Station" }, cookie);
  if (single.status !== 201 || !single.body?.id) {
    throw new Error(
      `create single-channel guild failed (${single.status}): ${JSON.stringify(single.body)}`,
    );
  }

  // 3) GUILD B — multi channel: default `general` + two more = 3 text channels.
  const multi = await postJson("/guilds", { name: "Orbit Hub" }, cookie);
  if (multi.status !== 201 || !multi.body?.id) {
    throw new Error(
      `create multi-channel guild failed (${multi.status}): ${JSON.stringify(multi.body)}`,
    );
  }
  for (const name of ["alpha", "beta"]) {
    const ch = await postJson(
      `/guilds/${multi.body.id}/channels`,
      { name, kind: "text" },
      cookie,
    );
    if (ch.status !== 201) {
      throw new Error(
        `create channel '${name}' failed (${ch.status}): ${JSON.stringify(ch.body)}`,
      );
    }
  }

  return {
    username,
    password,
    token,
    single: { id: single.body.id, name: single.body.name },
    multi: { id: multi.body.id, name: multi.body.name },
  };
}

// Allow `node seed.mjs` as a standalone smoke of the seed path.
if (import.meta.url === `file://${process.argv[1]}`) {
  seed()
    .then((s) => {
      console.log("seed OK:", {
        username: s.username,
        single: s.single,
        multi: s.multi,
        token: `${s.token.slice(0, 8)}…`,
      });
    })
    .catch((e) => {
      console.error("seed FAILED:", e.message);
      process.exit(1);
    });
}
