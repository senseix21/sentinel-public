export class ApiError extends Error {
  constructor(status, code, detail) {
    super(code);
    this.status = status;
    this.code = code;
    this.detail = detail;
  }
}

export async function api(path, options) {
  const response = await fetch(path, Object.assign({ headers: { Accept: "application/json" } }, options || {}));
  if (response.status === 401 && response.headers.get("X-Sentinel-Login")) {
    window.location.assign(response.headers.get("X-Sentinel-Login"));
    throw new ApiError(401, "session_expired");
  }
  const text = await response.text();
  let body = null;
  try { body = text ? JSON.parse(text) : null; } catch (_error) { body = null; }
  if (!response.ok) {
    throw new ApiError(response.status, (body && body.error) || `http_${response.status}`, body && body.detail);
  }
  return body;
}

export async function apiText(path) {
  const response = await fetch(path);
  if (!response.ok) throw new ApiError(response.status, `http_${response.status}`);
  return response.text();
}
