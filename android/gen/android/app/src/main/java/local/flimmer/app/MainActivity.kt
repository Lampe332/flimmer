package local.flimmer.app

import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.Color
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.ConsoleMessage
import android.webkit.JavascriptInterface
import android.webkit.GeolocationPermissions
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.widget.FrameLayout
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
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

  /**
   * The WebView's own fullscreen view answers Back itself (it leaves fullscreen) before the callback above runs.
   * In fullscreen the activity takes Back first: as a key (classic Back) and as an overlay callback (predictive Back).
   */
  private val overlayBack: Any? =
    if (Build.VERSION.SDK_INT >= 33) OnBackInvokedCallback { fullscreenBack.handleOnBackPressed() } else null

  override fun dispatchKeyEvent(event: KeyEvent): Boolean {
    if (fullscreenView != null && event.keyCode == KeyEvent.KEYCODE_BACK) {
      if (event.action == KeyEvent.ACTION_UP && !event.isCanceled) fullscreenBack.handleOnBackPressed()
      return true
    }
    return super.dispatchKeyEvent(event)
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

  /**
   * The phone player's brightness swipe (left half): `window.FlimmerScreen` sets this window's brightness;
   * a negative value gives it back to the system (the player does that when it closes).
   */
  private inner class ScreenBridge {
    @JavascriptInterface
    fun getBrightness(): Float {
      val own = window.attributes.screenBrightness
      if (own >= 0f) return own
      return try {
        Settings.System.getInt(contentResolver, Settings.System.SCREEN_BRIGHTNESS) / 255f
      } catch (e: Exception) {
        0.5f
      }
    }

    /** The phone player's volume swipe (right half): the system's media volume, 0 to 1, without the system slider. */
    @JavascriptInterface
    fun getVolume(): Float {
      val audio = getSystemService(AUDIO_SERVICE) as AudioManager
      val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
      return if (max > 0) audio.getStreamVolume(AudioManager.STREAM_MUSIC).toFloat() / max else 1f
    }

    @JavascriptInterface
    fun setVolume(value: Float) {
      val audio = getSystemService(AUDIO_SERVICE) as AudioManager
      val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
      val step = Math.round(value.coerceIn(0f, 1f) * max)
      try {
        if (audio.getStreamVolume(AudioManager.STREAM_MUSIC) != step) audio.setStreamVolume(AudioManager.STREAM_MUSIC, step, 0)
      } catch (e: SecurityException) {
        // Do Not Disturb can forbid changing a stream: the volume stays as it is
      }
    }

    @JavascriptInterface
    fun setBrightness(value: Float) {
      runOnUiThread {
        val attrs = window.attributes
        attrs.screenBrightness =
          if (value < 0f) WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE else value.coerceIn(0.01f, 1f)
        window.attributes = attrs
      }
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(ScreenBridge(), "FlimmerScreen")
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
    if (Build.VERSION.SDK_INT >= 33) {
      onBackInvokedDispatcher.registerOnBackInvokedCallback(
        OnBackInvokedDispatcher.PRIORITY_OVERLAY, overlayBack as OnBackInvokedCallback
      )
    }
  }

  private fun leaveFullscreen() {
    val view = fullscreenView ?: return
    (window.decorView as ViewGroup).removeView(view)
    fullscreenView = null
    fullscreenBack.remove()
    if (Build.VERSION.SDK_INT >= 33) onBackInvokedDispatcher.unregisterOnBackInvokedCallback(overlayBack as OnBackInvokedCallback)
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
