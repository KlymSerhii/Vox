/**
 * Platform detection for render-time branching.
 *
 * The host OS never changes during a session, so we sniff the WebView's
 * user-agent string once at module load — WKWebView reports "Macintosh",
 * WebView2 reports "Windows NT". This stays synchronous (usable directly
 * in JSX) and adds no dependency or Tauri capability, unlike the async
 * `@tauri-apps/plugin-os` `platform()`.
 */
export const isMac = /Macintosh|Mac OS X/.test(navigator.userAgent);
export const isWindows = /Windows NT/.test(navigator.userAgent);

/**
 * User-facing device noun for copy that addresses the user's machine
 * ("runs entirely on this Mac" → "this PC" on Windows). Already
 * capitalized, so it drops straight into a sentence.
 */
export const deviceWord = isMac ? "Mac" : "PC";
