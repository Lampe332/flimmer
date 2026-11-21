package local.flimmer.app

import android.annotation.SuppressLint
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Message
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayInputStream

@InvokeArg
class OpenArgs {
  var id: Int = 0
  lateinit var url: String
  lateinit var channel: Channel
}

@InvokeArg
class PageArgs {
  var id: Int = 0
}

@InvokeArg
class ExternalArgs {
  lateinit var url: String
  var mime: String? = null
}

/**
 * The Android side of the stream sniffer (android/src/sniffer.rs decides, this class only watches):
 * one WebView per provider page, laid out behind the app's own WebView so the page believes it is visible.
 * Every media-looking request of any frame is reported with its request headers. Popups, foreign top-level
 * navigations, downloads, dialogs and permission prompts are denied, ads are blocked by the Rust ad blocker.
 */
@TauriPlugin
class SnifferPlugin(private val activity: Activity) : Plugin(activity) {
  private val pages = HashMap<Int, ScanPage>()

  @Command
  fun open(invoke: Invoke) {
    val args = invoke.parseArgs(OpenArgs::class.java)
    activity.runOnUiThread {
      try {
        pages.remove(args.id)?.close()
        pages[args.id] = ScanPage(activity, args.url, args.channel)
        invoke.resolve()
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  /** A real tap in the middle of the page; reaches cross-origin iframes too. */
  @Command
  fun click(invoke: Invoke) {
    val args = invoke.parseArgs(PageArgs::class.java)
    activity.runOnUiThread {
      pages[args.id]?.tapCentre()
      invoke.resolve()
    }
  }

  @Command
  fun close(invoke: Invoke) {
    val args = invoke.parseArgs(PageArgs::class.java)
    activity.runOnUiThread {
      pages.remove(args.id)?.close()
      invoke.resolve()
    }
  }

  /** Cookies and HTTP cache of all WebViews ("Cache leeren" in the settings). The UI's own storage stays. */
  @Command
  fun clearBrowserData(invoke: Invoke) {
    activity.runOnUiThread {
      CookieManager.getInstance().removeAllCookies(null)
      val probe = WebView(activity)
      probe.clearCache(true)
      probe.destroy()
      invoke.resolve()
    }
  }

  /** A web page in the browser (no `mime`), or a media stream in a video player app. */
  @Command
  fun openExternal(invoke: Invoke) {
    val args = invoke.parseArgs(ExternalArgs::class.java)
    val uri = Uri.parse(args.url)
    if (uri.scheme != "http" && uri.scheme != "https") {
      invoke.reject("only web addresses can be opened")
      return
    }
    val intent = Intent(Intent.ACTION_VIEW).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (args.mime == null) intent.data = uri else intent.setDataAndType(uri, args.mime)
    try {
      activity.startActivity(intent)
      invoke.resolve()
    } catch (e: ActivityNotFoundException) {
      invoke.reject(if (args.mime == null) "no browser installed" else "no video player app installed")
    }
  }

  override fun onDestroy(activity: AppCompatActivity) {
    pages.values.forEach { it.close() }
    pages.clear()
  }
}

/** Runs in every frame before the page's own scripts: no sound, and paused videos are started. */
private const val PAGE_SCRIPT = """(() => {
  try {
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () { this.muted = true; return play.apply(this, arguments); };
  } catch (e) {}
  let n = 0;
  const tick = () => {
    try {
      for (const v of document.querySelectorAll('video,audio')) {
        v.muted = true;
        if (v.tagName === 'VIDEO' && v.paused) { const p = v.play(); if (p) p.catch(() => {}); }
      }
    } catch (e) {}
    if (++n < 12) setTimeout(tick, 2000);
  };
  setTimeout(tick, 2500);
})();"""

@SuppressLint("SetJavaScriptEnabled")
private class ScanPage(private val activity: Activity, private val url: String, private val channel: Channel) {
  private val web = WebView(activity)
  private val root: ViewGroup = activity.findViewById(android.R.id.content)

  /** Hosts the top-level page may be on: the provider's and those it is redirected to. */
  private val hosts = HashSet<String>()
  private val seen = HashSet<String>()

  @Volatile private var pageUrl = url
  @Volatile private var closed = false
  private var loaded = false
  private var failed = false
  private val userAgent: String

  init {
    Uri.parse(url).host?.let { hosts.add(it) }
    val s = web.settings
    // A plain Chrome for Android: provider pages treat an app's WebView differently.
    userAgent = s.userAgentString.replace("; wv", "").replace(Regex("Version/\\d+\\.\\d+ "), "")
    s.userAgentString = userAgent
    s.javaScriptEnabled = true
    s.domStorageEnabled = true
    s.mediaPlaybackRequiresUserGesture = false
    s.javaScriptCanOpenWindowsAutomatically = false
    s.setSupportMultipleWindows(true) // so popups arrive in onCreateWindow (and are denied there)
    s.mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
    s.setGeolocationEnabled(false)
    if (WebViewFeature.isFeatureSupported(WebViewFeature.REQUESTED_WITH_HEADER_ALLOW_LIST)) {
      // No "X-Requested-With: local.flimmer.app" on the page's requests.
      WebSettingsCompat.setRequestedWithHeaderOriginAllowList(s, emptySet())
    }
    if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
      WebViewCompat.addDocumentStartJavaScript(web, PAGE_SCRIPT, setOf("*"))
    }
    CookieManager.getInstance().setAcceptThirdPartyCookies(web, true)
    web.isFocusable = false
    web.isFocusableInTouchMode = false
    web.isHapticFeedbackEnabled = false
    web.isSoundEffectsEnabled = false
    web.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
    web.setDownloadListener { _, _, _, _, _ -> send("note", "text", "download denied") }
    web.webViewClient = Client()
    web.webChromeClient = Chrome()
    // Index 0: drawn first, so the app's own WebView covers it. Attached and "visible", or players do not start.
    root.addView(web, 0, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
    web.loadUrl(url)
  }

  fun close() {
    if (closed) return
    closed = true
    web.stopLoading()
    root.removeView(web)
    web.destroy()
  }

  fun tapCentre() {
    if (closed || web.width == 0) return
    val (x, y) = web.width / 2f to web.height / 2f
    val t = SystemClock.uptimeMillis()
    for ((action, at) in listOf(MotionEvent.ACTION_DOWN to t, MotionEvent.ACTION_UP to t + 60)) {
      val ev = MotionEvent.obtain(t, at, action, x, y, 0)
      web.dispatchTouchEvent(ev)
      ev.recycle()
    }
  }

  private fun send(event: String, key: String? = null, value: String? = null, headers: Map<String, String>? = null) {
    if (closed) return
    val o = JSObject()
    o.put("e", event)
    if (key != null) o.put(key, value)
    if (headers != null) {
      val h = JSObject()
      for ((k, v) in headers) h.put(k, v)
      o.put("headers", h)
    }
    channel.send(o)
  }

  private fun looksLikeMedia(u: String): Boolean {
    val l = u.lowercase()
    return l.contains(".m3u") || l.contains(".mp4") || l.contains(".mpd")
  }

  /** The request type as the filter lists name it, guessed from what Android tells about a request. */
  private fun kind(request: WebResourceRequest): String {
    val accept = request.requestHeaders["Accept"] ?: ""
    val path = (request.url.path ?: "").lowercase()
    return when {
      accept.startsWith("text/html") -> "sub_frame"
      accept.startsWith("text/css") || path.endsWith(".css") -> "stylesheet"
      accept.startsWith("image/") || path.matches(Regex(".*\\.(png|jpe?g|gif|webp|svg|ico|avif)$")) -> "image"
      path.endsWith(".js") || path.endsWith(".mjs") -> "script"
      path.matches(Regex(".*\\.(woff2?|ttf|otf)$")) -> "font"
      path.matches(Regex(".*\\.(mp4|m3u8|ts|m4s|webm|mp3|aac)$")) -> "media"
      request.requestHeaders.containsKey("Origin") || accept.contains("json") -> "xhr"
      else -> "other"
    }
  }

  private inner class Client : WebViewClient() {
    // Called on a network thread for every request of every frame and worker.
    override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
      val u = request.url.toString()
      if (!u.startsWith("http")) return null
      if (request.isForMainFrame) {
        pageUrl = u
        return null
      }
      if (Adblock.blocked(u, pageUrl, kind(request))) {
        return WebResourceResponse("text/plain", "utf-8", 204, "No Content", emptyMap(), ByteArrayInputStream(ByteArray(0)))
      }
      if (looksLikeMedia(u) && synchronized(seen) { seen.add(u) }) {
        val headers = HashMap(request.requestHeaders)
        if (headers.keys.none { it.equals("User-Agent", true) }) headers["User-Agent"] = userAgent
        send("req", "url", u, headers)
      }
      return null
    }

    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
      val u = request.url
      val http = u.scheme == "http" || u.scheme == "https"
      if (!request.isForMainFrame) return !http // frames stay in the web: no intent:, market:, tel: ...
      val host = u.host ?: ""
      if (http && (hosts.contains(host) || request.isRedirect)) {
        hosts.add(host) // a server redirect of an allowed navigation may move to another host
        return false
      }
      send("note", "text", "navigation blocked " + u.toString().take(160))
      return true
    }

    override fun onPageFinished(view: WebView, url: String) {
      if (!failed && !loaded && url.startsWith("http")) {
        loaded = true
        send("loaded")
      }
    }

    override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
      // Only plain network failures of the provider page itself end the scan early.
      if (!request.isForMainFrame || loaded || failed) return
      val reason = when (error.errorCode) {
        ERROR_HOST_LOOKUP -> "provider host not found"
        ERROR_TIMEOUT -> "provider page timed out"
        ERROR_CONNECT, ERROR_IO -> "provider page not reachable"
        else -> return
      }
      failed = true
      send("fail", "reason", reason)
    }

    override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
      send("fail", "reason", "provider page crashed")
      return true // handled: the app itself keeps running
    }
  }

  private inner class Chrome : WebChromeClient() {
    override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
      send("popup")
      return false
    }

    override fun onJsAlert(view: WebView, url: String, message: String, result: JsResult): Boolean {
      result.cancel()
      return true
    }

    override fun onJsConfirm(view: WebView, url: String, message: String, result: JsResult): Boolean {
      result.cancel()
      return true
    }

    override fun onJsPrompt(view: WebView, url: String, message: String, defaultValue: String?, result: JsPromptResult): Boolean {
      result.cancel()
      return true
    }

    override fun onJsBeforeUnload(view: WebView, url: String, message: String, result: JsResult): Boolean {
      result.confirm()
      return true
    }

    override fun onPermissionRequest(request: PermissionRequest) {
      request.deny()
    }

    // Fullscreen requests of the page's player are turned down at once.
    override fun onShowCustomView(view: View, callback: CustomViewCallback) {
      callback.onCustomViewHidden()
    }
  }
}
