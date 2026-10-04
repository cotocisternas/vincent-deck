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

void vincent_audio_label(Audio *a, guint32 id, char *label, size_t capacity) {
  if (capacity == 0)
    return;
  label[0] = '\0';
  if (!a->nodes || a->failed)
    return;
  g_autoptr(WpNode) node = wp_object_manager_lookup(a->nodes, WP_TYPE_NODE,
      WP_CONSTRAINT_TYPE_G_PROPERTY, "bound-id", "=u", id, NULL);
  if (!node)
    return;
  const char *name = wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "node.nick");
  if (!name || !*name)
    name = wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "node.description");
  if (!name || !*name)
    name = wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "node.name");
  if (name)
    g_strlcpy(label, name, capacity);
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

static gboolean synchronize(Audio *a, GCancellable *cancel) {
  /* The first barrier delivers Props/Route events; the second lets the mixer
   * complete its own sync and update its cache before another adjustment. */
  gboolean ok = TRUE;
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
  a->dirty = TRUE;
  return ok;
}

static GSource *write_deadline(Audio *a, GCancellable *cancel) {
  GSource *deadline = g_timeout_source_new(2000);
  g_source_set_callback(deadline, expired, cancel, NULL);
  g_source_attach(deadline, a->context);
  return deadline;
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
  g_autoptr(GCancellable) cancel = g_cancellable_new();
  GSource *deadline = write_deadline(a, cancel);
  ok = synchronize(a, cancel);
  g_source_destroy(deadline);
  g_source_unref(deadline);
  return ok;
}

static gint compare_nodes(gconstpointer first, gconstpointer second) {
  WpPipewireObject *a = *(WpPipewireObject * const *)first;
  WpPipewireObject *b = *(WpPipewireObject * const *)second;
  return g_strcmp0(wp_pipewire_object_get_property(a, "node.name"),
      wp_pipewire_object_get_property(b, "node.name"));
}

int vincent_audio_cycle(Audio *a, int source) {
  if (a->failed || a->pending || !a->defaults || !a->nodes)
    return FALSE;
  const char *media_class = source ? "Audio/Source" : "Audio/Sink";
  g_autoptr(GPtrArray) nodes = g_ptr_array_new_with_free_func(g_object_unref);
  g_autoptr(WpIterator) iterator = wp_object_manager_new_filtered_iterator(a->nodes,
      WP_TYPE_NODE, WP_CONSTRAINT_TYPE_PW_PROPERTY, "media.class", "=s", media_class, NULL);
  GValue value = G_VALUE_INIT;
  while (wp_iterator_next(iterator, &value)) {
    WpNode *node = g_value_get_object(&value);
    const char *name = wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "node.name");
    if (name && *name)
      g_ptr_array_add(nodes, g_object_ref(node));
    g_value_unset(&value);
  }
  if (nodes->len == 0)
    return FALSE;
  g_ptr_array_sort(nodes, compare_nodes);
  guint32 current = G_MAXUINT32;
  g_signal_emit_by_name(a->defaults, "get-default-node", media_class, &current);
  guint next = 0;
  for (guint i = 0; i < nodes->len; i++) {
    if (wp_proxy_get_bound_id(WP_PROXY(nodes->pdata[i])) == current) {
      next = (i + 1) % nodes->len;
      break;
    }
  }
  WpNode *node = nodes->pdata[next];
  guint32 target = wp_proxy_get_bound_id(WP_PROXY(node));
  if (target == current)
    return TRUE;
  const char *name = wp_pipewire_object_get_property(WP_PIPEWIRE_OBJECT(node), "node.name");
  gboolean ok = FALSE;
  /* Sticky selection: WirePlumber policy applies it to default.audio.* and
   * existing streams, just as a system default-device selection does. */
  g_signal_emit_by_name(a->defaults, "set-default-configured-node-name", media_class, name, &ok);
  if (!ok)
    return FALSE;
  g_autoptr(GCancellable) cancel = g_cancellable_new();
  GSource *deadline = write_deadline(a, cancel);
  ok = synchronize(a, cancel);
  while (ok && !a->failed && !g_cancellable_is_cancelled(cancel)) {
    g_signal_emit_by_name(a->defaults, "get-default-node", media_class, &current);
    if (current == target)
      break;
    g_main_context_iteration(a->context, TRUE);
  }
  ok = ok && !a->failed && current == target;
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
