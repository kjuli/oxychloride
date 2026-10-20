/*
 * Oxychloride — OCL for Ecore models. C ABI.
 *
 * Link against liboxychloride_ffi (cargo build -p oxy-ffi --release → target/release).
 *
 * Usage: create a session, send JSON-RPC 2.0 requests (see docs/PROTOCOL.md), free every
 * returned string with oxy_string_free(). Sessions are not thread-safe.
 *
 *   OxySession *s = oxy_session_new();
 *   char *r = oxy_session_call(s, "loadEcore", "{\"path\": \"library.ecore\"}");
 *   ... parse r as JSON: {"result": ...} or {"error": {"code": ..., "message": ...}} ...
 *   oxy_string_free(r);
 *   oxy_session_free(s);
 */
#ifndef OXYCHLORIDE_H
#define OXYCHLORIDE_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct OxySession OxySession;

/* Creates a new, empty session. */
OxySession *oxy_session_new(void);

/* Releases a session. NULL is allowed. */
void oxy_session_free(OxySession *session);

/* Handles a JSON-RPC request or batch; returns the JSON response, or NULL for
 * notifications (requests without "id"). */
char *oxy_session_request(OxySession *session, const char *request_json);

/* Calls `method` with `params_json` (may be NULL). Returns {"result": ...} or
 * {"error": {"code": ..., "message": ...}}. */
char *oxy_session_call(OxySession *session, const char *method, const char *params_json);

/* Releases a string returned by this library. NULL is allowed. */
void oxy_string_free(char *s);

/* Library version (static string, do not free). */
const char *oxy_version(void);

#ifdef __cplusplus
}
#endif

#endif /* OXYCHLORIDE_H */
