package local.flimmer.app

/** The ad blocker lives in the Rust library (Brave's `adblock` engine, see android/src/adblock.rs). */
object Adblock {
  /**
   * Whether the filter lists block this request. `source` is the page that asks, `kind` a request type the
   * lists know (script, image, sub_frame, xhr, ...). False while the lists are still loading.
   */
  @JvmStatic
  external fun blocked(url: String, source: String, kind: String): Boolean
}
