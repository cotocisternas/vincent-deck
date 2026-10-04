/* Native WirePlumber 0.5 bridge. All objects and callbacks belong to the
 * calling audio thread's GLib context. No audio commands are spawned. */
#include <wp/wp.h>
#include <math.h>

typedef struct {
  WpCore *core;
  WpPlugin *defaults;
  WpPlugin *mixer;
  WpObjectManager *nodes;
  GCancellable *cancel;
  GSource *startup_deadline;
  GMainContext *context;
  guint pending;
  gboolean failed;
  gboolean dirty;
} Audio;

typedef struct {
  guint32 id;
  double volume;
  int muted;
  int known;
} AudioValue;

static void disconnected(WpCore *core, Audio *a) {
  (void) core;
  a->failed = TRUE;
  a->dirty = TRUE;
}

static void changed(WpPlugin *plugin, Audio *a) {
  (void) plugin;
  a->dirty = TRUE;
}

static void mixer_changed(WpPlugin *plugin, guint id, Audio *a) {
  (void) id;
  changed(plugin, a);
}

static void nodes_changed(WpObjectManager *nodes, Audio *a) {
  (void) nodes;
  a->dirty = TRUE;
}

static gboolean startup_expired(gpointer data) {
  Audio *a = data;
  if (a->pending) {
    a->failed = TRUE;
    g_cancellable_cancel(a->cancel);
  }
  return G_SOURCE_REMOVE;
}

static void activated(GObject *object, GAsyncResult *result, gpointer data) {
  Audio *a = data;
  GError *error = NULL;
  if (!wp_object_activate_finish(WP_OBJECT(object), result, &error)) {
    a->failed = TRUE;
    g_clear_error(&error);
  }
  a->pending--;
  a->dirty = TRUE;
}

static void loaded(GObject *object, GAsyncResult *result, gpointer data) {
  Audio *a = data;
  GError *error = NULL;
  if (!wp_core_load_component_finish(WP_CORE(object), result, &error)) {
    a->failed = TRUE;
    g_clear_error(&error);
  }
  a->pending--;
  if (a->pending || a->failed)
    return;
  a->defaults = wp_plugin_find(a->core, "default-nodes-api");
  a->mixer = wp_plugin_find(a->core, "mixer-api");
  if (!a->defaults || !a->mixer) {
    a->failed = TRUE;
    return;
  }
  /* Match wpctl's user-facing cubic scale and hardware-route semantics. */
  g_object_set(a->mixer, "scale", 1, NULL);
  g_signal_connect(a->defaults, "changed", G_CALLBACK(changed), a);
  g_signal_connect(a->mixer, "changed", G_CALLBACK(mixer_changed), a);
  a->pending = 2;
  wp_object_activate(WP_OBJECT(a->defaults), WP_PLUGIN_FEATURE_ENABLED,
      a->cancel, activated, a);
  wp_object_activate(WP_OBJECT(a->mixer), WP_PLUGIN_FEATURE_ENABLED,
      a->cancel, activated, a);
}

Audio *vincent_audio_new(GMainContext *context, const char *remote) {
  static gsize initialized;
  if (g_once_init_enter(&initialized)) {
    wp_init(WP_INIT_PIPEWIRE | WP_INIT_SPA_TYPES);
    g_once_init_leave(&initialized, 1);
  }
  Audio *a = g_new0(Audio, 1);
  a->context = context;
  a->cancel = g_cancellable_new();
  a->dirty = TRUE;
  WpProperties *properties = wp_properties_new("application.name", "Vincent Deck", NULL);
  if (remote)
    wp_properties_set(properties, "remote.name", remote);
  a->core = wp_core_new(context, NULL, properties);
  g_signal_connect(a->core, "disconnected", G_CALLBACK(disconnected), a);
  if (!wp_core_connect(a->core)) {
    a->failed = TRUE;
    return a;
  }
  a->pending = 2;
  a->nodes = wp_object_manager_new();
  wp_object_manager_add_interest(a->nodes, WP_TYPE_NODE, NULL);
  wp_object_manager_request_object_features(a->nodes, WP_TYPE_NODE,
      WP_PIPEWIRE_OBJECT_FEATURES_MINIMAL);
  g_signal_connect(a->nodes, "objects-changed", G_CALLBACK(nodes_changed), a);
  wp_core_install_object_manager(a->core, a->nodes);
  a->startup_deadline = g_timeout_source_new(2000);
  g_source_set_callback(a->startup_deadline, startup_expired, a, NULL);
  g_source_attach(a->startup_deadline, context);
  wp_core_load_component(a->core, "libwireplumber-module-default-nodes-api",
      "module", NULL, NULL, a->cancel, loaded, a);
  wp_core_load_component(a->core, "libwireplumber-module-mixer-api",
      "module", NULL, NULL, a->cancel, loaded, a);
  return a;
}

int vincent_audio_failed(Audio *a) { return a->failed; }

int vincent_audio_dirty(Audio *a) {
  gboolean dirty = a->dirty;
  a->dirty = FALSE;
  return dirty;
}

AudioValue vincent_audio_read(Audio *a, int source) {
  AudioValue value = {0};
  if (a->failed || a->pending || !a->defaults || !a->mixer)
    return value;
  g_signal_emit_by_name(a->defaults, "get-default-node",
      source ? "Audio/Source" : "Audio/Sink", &value.id);
  if (value.id == 0 || value.id == G_MAXUINT32)
    return value;
  g_autoptr(GVariant) dict = NULL;
  gboolean mute;
  g_signal_emit_by_name(a->mixer, "get-volume", value.id, &dict);
  if (dict && g_variant_lookup(dict, "volume", "d", &value.volume) &&
      g_variant_lookup(dict, "mute", "b", &mute) &&
      isfinite(value.volume) && value.volume >= 0.0) {
    value.muted = mute;
    value.known = TRUE;
  }
  return value;
}

typedef struct { gboolean done; gboolean ok; } Sync;

static void synced(GObject *object, GAsyncResult *result, gpointer data) {
  Sync *s = data;
  GError *error = NULL;
  s->ok = wp_core_sync_finish(WP_CORE(object), result, &error);
  g_clear_error(&error);
  s->done = TRUE;
}

static gboolean expired(gpointer data) {
  g_cancellable_cancel(data);
  return G_SOURCE_REMOVE;
}

int vincent_audio_write(Audio *a, guint32 id, double volume, int mute) {
  if (a->failed || a->pending || !a->mixer)
    return FALSE;
  GVariantBuilder builder;
  g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
  if (mute < 0)
    g_variant_builder_add(&builder, "{sv}", "volume", g_variant_new_double(volume));
  else
    g_variant_builder_add(&builder, "{sv}", "mute", g_variant_new_boolean(mute));
  g_autoptr(GVariant) dict = g_variant_ref_sink(g_variant_builder_end(&builder));
  gboolean ok = FALSE;
  g_signal_emit_by_name(a->mixer, "set-volume", id, dict, &ok);
  if (!ok)
    return FALSE;
  /* The first barrier delivers Props/Route events; the second lets the mixer
   * complete its own sync and update its cache before another adjustment. */
  g_autoptr(GCancellable) cancel = g_cancellable_new();
  GSource *deadline = g_timeout_source_new(2000);
  g_source_set_callback(deadline, expired, cancel, NULL);
  g_source_attach(deadline, a->context);
  for (int i = 0; i < 2 && ok; i++) {
    Sync sync = {0};
    if (!wp_core_sync(a->core, cancel, synced, &sync)) {
      ok = FALSE;
      break;
    }
    while (!sync.done)
      g_main_context_iteration(a->context, TRUE);
    ok = sync.ok && !a->failed;
  }
  g_source_destroy(deadline);
  g_source_unref(deadline);
  a->dirty = TRUE;
  return ok;
}

void vincent_audio_free(Audio *a) {
  if (a->startup_deadline) {
    g_source_destroy(a->startup_deadline);
    g_source_unref(a->startup_deadline);
  }
  g_cancellable_cancel(a->cancel);
  wp_core_disconnect(a->core);
  /* Keep callback userdata alive until cancellation callbacks have finished. */
  while (a->pending)
    g_main_context_iteration(a->context, TRUE);
  if (a->mixer)
    g_signal_handlers_disconnect_by_data(a->mixer, a);
  if (a->defaults)
    g_signal_handlers_disconnect_by_data(a->defaults, a);
  g_signal_handlers_disconnect_by_data(a->core, a);
  if (a->nodes)
    g_signal_handlers_disconnect_by_data(a->nodes, a);
  g_clear_object(&a->nodes);
  g_clear_object(&a->mixer);
  g_clear_object(&a->defaults);
  g_clear_object(&a->core);
  g_clear_object(&a->cancel);
  g_free(a);
}
