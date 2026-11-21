package local.flimmer.app

import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  private var fullscreenView: View? = null
  private var fullscreenCallback: WebChromeClient.CustomViewCallback? = null

  /**
   * Phones show the app upright and only the fullscreen player in landscape; tablets turn freely.
   * (With free rotation a phone lying flat stays in landscape after fullscreen.)
   */
  private var baseOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED

  private var webView: WebView? = null

  /**
   * Back in fullscreen: an open sheet of the player closes first (the page answers `flimmerBack()` with true),
   * then fullscreen ends. Added on entering, so it comes before the WebView's "go back".
   */
  private val fullscreenBack = object : OnBackPressedCallback(true) {
    override fun handleOnBackPressed() {
      val view = webView ?: return leaveFullscreen()
      view.evaluateJavascript("!!(window.flimmerBack && window.flimmerBack())") { closed ->
        if (closed != "true") leaveFullscreen()
      }
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    if (resources.configuration.smallestScreenWidthDp < 600) baseOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
    requestedOrientation = baseOrientation
    val content = findViewById<ViewGroup>(android.R.id.content)
    // The page is not drawn under the system bars or the keyboard; the strip behind them has the app's colour.
    window.decorView.setBackgroundColor(Color.parseColor("#090c17"))
    WindowCompat.getInsetsController(window, window.decorView).apply {
      isAppearanceLightStatusBars = false
      isAppearanceLightNavigationBars = false
    }
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      )
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    clearCacheAfterUpdate(webView)
    // After the WebView has its own chrome client (file picker, dialogs): wrap it to add real fullscreen.
    webView.post {
      val inner = webView.webChromeClient
      if (inner != null && inner !is FullscreenChrome) webView.webChromeClient = FullscreenChrome(inner)
    }
  }

  /** First start after an install or update: drop the WebView's HTTP cache (old UI files). Cookies and storage stay. */
  private fun clearCacheAfterUpdate(webView: WebView) {
    val installed = try {
      packageManager.getPackageInfo(packageName, 0).lastUpdateTime
    } catch (e: Exception) {
      return
    }
    val prefs = getSharedPreferences("flimmer", MODE_PRIVATE)
    if (prefs.getLong("webCacheFor", 0L) == installed) return
    webView.clearCache(true)
    prefs.edit().putLong("webCacheFor", installed).apply()
  }

  private fun enterFullscreen(view: View, callback: WebChromeClient.CustomViewCallback) {
    if (fullscreenView != null) {
      callback.onCustomViewHidden()
      return
    }
    fullscreenView = view
    fullscreenCallback = callback
    view.setBackgroundColor(Color.BLACK)
    (window.decorView as ViewGroup).addView(
      view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
    )
    WindowCompat.getInsetsController(window, window.decorView).apply {
      systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      hide(WindowInsetsCompat.Type.systemBars())
    }
    requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
    onBackPressedDispatcher.addCallback(this, fullscreenBack)
  }

  private fun leaveFullscreen() {
    val view = fullscreenView ?: return
    (window.decorView as ViewGroup).removeView(view)
    fullscreenView = null
    fullscreenBack.remove()
    fullscreenCallback?.onCustomViewHidden()
    fullscreenCallback = null
    WindowCompat.getInsetsController(window, window.decorView).show(WindowInsetsCompat.Type.systemBars())
    requestedOrientation = baseOrientation
  }

  /** The WebView's own chrome client, plus fullscreen for the player (landscape, no system bars). */
  private inner class FullscreenChrome(private val inner: WebChromeClient) : WebChromeClient() {
    override fun onShowCustomView(view: View, callback: CustomViewCallback) = enterFullscreen(view, callback)
    override fun onHideCustomView() = leaveFullscreen()

    override fun onPermissionRequest(request: PermissionRequest) = inner.onPermissionRequest(request)
    override fun getDefaultVideoPoster(): Bitmap? = inner.defaultVideoPoster
    override fun onJsAlert(view: WebView, url: String, message: String, result: JsResult) =
      inner.onJsAlert(view, url, message, result)
    override fun onJsConfirm(view: WebView, url: String, message: String, result: JsResult) =
      inner.onJsConfirm(view, url, message, result)
    override fun onJsPrompt(view: WebView, url: String, message: String, defaultValue: String?, result: JsPromptResult) =
      inner.onJsPrompt(view, url, message, defaultValue, result)
    override fun onGeolocationPermissionsShowPrompt(origin: String, callback: GeolocationPermissions.Callback) =
      inner.onGeolocationPermissionsShowPrompt(origin, callback)
    override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams) =
      inner.onShowFileChooser(view, callback, params)
    override fun onConsoleMessage(message: ConsoleMessage) = inner.onConsoleMessage(message)
    override fun onReceivedTitle(view: WebView, title: String) = inner.onReceivedTitle(view, title)
  }
}
