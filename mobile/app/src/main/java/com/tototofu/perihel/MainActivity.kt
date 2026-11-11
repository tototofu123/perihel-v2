package com.tototofu.perihel

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import android.util.Base64
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { PerihelTheme { PerihelApp() } }
    }
}

private val Accent = Color(0xFF7C9EFF)
private val DoneGreen = Color(0xFF7BC98A)
private val Warn = Color(0xFFE0B36A)
private val Dim = Color(0xFF8A8A96)
private val Surface0 = Color(0xFF141419)
private val Surface1 = Color(0xFF1C1C23)

private val CATEGORIES = listOf("idea", "todo", "link", "note", "question", "person", "event")
private val TYPES = listOf("text", "url", "image")

@Composable
fun PerihelTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(primary = Accent, background = Surface0, surface = Surface0),
        content = content,
    )
}

class Config(ctx: Context) {
    private val prefs = ctx.getSharedPreferences("perihel", Context.MODE_PRIVATE)
    var url: String
        get() = prefs.getString("url", BuildConfig.DEFAULT_URL) ?: ""
        set(v) = prefs.edit().putString("url", v).apply()
    var token: String
        get() = prefs.getString("token", BuildConfig.DEFAULT_TOKEN) ?: ""
        set(v) = prefs.edit().putString("token", v).apply()

    // chat history survives app restarts (last 100 messages, JSON in prefs)
    fun loadChat(): List<ChatMsg> = try {
        val arr = org.json.JSONArray(prefs.getString("chat", "[]"))
        (0 until arr.length()).map {
            val o = arr.getJSONObject(it)
            ChatMsg(o.getString("who"), o.getString("text"), o.optString("extra"))
        }
    } catch (_: Exception) { emptyList() }

    fun saveChat(messages: List<ChatMsg>) {
        val arr = org.json.JSONArray()
        messages.takeLast(100).forEach {
            arr.put(org.json.JSONObject().put("who", it.who).put("text", it.text)
                .put("extra", it.extra))
        }
        prefs.edit().putString("chat", arr.toString()).apply()
    }

    // thoughts captured while the server was down, waiting to sync (cap 10)
    fun loadPending(): List<SyncApi.PendingSend> = try {
        val arr = org.json.JSONArray(prefs.getString("pending", "[]"))
        (0 until arr.length()).map {
            val o = arr.getJSONObject(it)
            SyncApi.PendingSend(
                o.getString("uuid"), o.getString("text"),
                if (o.isNull("thumb")) null else o.optString("thumb"),
                o.optString("created_at"),
                if (o.isNull("ai_summary")) null else o.optString("ai_summary"),
                if (o.isNull("category")) null else o.optString("category"))
        }
    } catch (_: Exception) { emptyList() }

    fun savePending(list: List<SyncApi.PendingSend>) {
        val arr = org.json.JSONArray()
        list.takeLast(10).forEach { p ->
            arr.put(org.json.JSONObject()
                .put("uuid", p.uuid).put("text", p.text)
                .put("thumb", p.thumbDataUri ?: org.json.JSONObject.NULL)
                .put("created_at", p.createdAt)
                .put("ai_summary", p.aiSummary ?: org.json.JSONObject.NULL)
                .put("category", p.category ?: org.json.JSONObject.NULL))
        }
        prefs.edit().putString("pending", arr.toString()).apply()
    }

    val isConfigured: Boolean get() = url.isNotBlank() && token.isNotBlank()
}

@Composable
fun PerihelApp() {
    val ctx = LocalContext.current
    val config = remember { Config(ctx) }
    var screen by remember { mutableStateOf(if (config.isConfigured) "main" else "setup") }
    var tab by remember { mutableIntStateOf(0) }

    if (screen == "setup") {
        SetupScreen(config) { screen = "main" }
        return
    }

    Scaffold(
        bottomBar = {
            NavigationBar(containerColor = Surface1) {
                listOf("💬 Capture", "📖 Logbook", "🔔 Alerts").forEachIndexed { i, label ->
                    NavigationBarItem(
                        selected = tab == i,
                        onClick = { tab = i },
                        icon = {},
                        label = { Text(label, fontSize = 12.sp,
                            color = if (tab == i) Accent else Dim) },
                    )
                }
            }
        },
    ) { padding ->
        Box(Modifier.padding(padding)) {
            when (tab) {
                0 -> ChatTab(config, onDisconnect = { screen = "setup" })
                1 -> LogbookTab(config)
                2 -> AlertsTab(config)
            }
        }
    }
}

// ---------------- setup ----------------

@Composable
fun SetupScreen(config: Config, onDone: () -> Unit) {
    val scope = rememberCoroutineScope()
    var url by remember { mutableStateOf(config.url) }
    var token by remember { mutableStateOf(config.token) }
    var message by remember { mutableStateOf("") }

    Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center) {
        Text("Perihel", fontSize = 28.sp, color = Accent)
        Text("connect to your sync server", fontSize = 13.sp, color = Dim)
        Spacer(Modifier.height(24.dp))
        OutlinedTextField(url, { url = it }, label = { Text("Server URL") },
            singleLine = true, modifier = Modifier.fillMaxWidth())
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(token, { token = it }, label = { Text("Token") },
            singleLine = true, modifier = Modifier.fillMaxWidth())
        Spacer(Modifier.height(16.dp))
        if (message.isNotEmpty()) Text(message, fontSize = 12.sp, color = MaterialTheme.colorScheme.error)
        Button(onClick = {
            scope.launch {
                message = "checking…"
                try {
                    withContext(Dispatchers.IO) { SyncApi.health(url, token) }
                    config.url = url.trim(); config.token = token.trim()
                    onDone()
                } catch (e: Exception) {
                    android.util.Log.e("SyncApi", "connect failed", e)
                    message = e.message ?: e.toString()
                }
            }
        }, modifier = Modifier.fillMaxWidth()) { Text("Connect") }
    }
}

// ---------------- tab 1: chat ----------------

data class ChatMsg(val who: String, val text: String, val extra: String = "",
                   val image: Bitmap? = null)  // image previews are session-only

/** A photo picked from the gallery, ready to attach: raw bytes for upload,
 * a downscaled base64 thumbnail for the vision model, and the preview bitmap. */
data class PendingImage(val bytes: ByteArray, val filename: String,
                        val thumbDataUri: String, val preview: Bitmap)

/** Decode the picked uri down to ~1280px so the vision turn stays light. */
suspend fun prepareImage(ctx: Context, uri: Uri): PendingImage? =
    withContext(Dispatchers.IO) {
        try {
            val resolver = ctx.contentResolver
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            resolver.openInputStream(uri)?.use {
                BitmapFactory.decodeStream(it, null, bounds)
            }
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= 1280) sample *= 2
            val opts = BitmapFactory.Options().apply { inSampleSize = sample }
            val bmp = resolver.openInputStream(uri)?.use {
                BitmapFactory.decodeStream(it, null, opts)
            } ?: return@withContext null
            val baos = java.io.ByteArrayOutputStream()
            bmp.compress(Bitmap.CompressFormat.JPEG, 80, baos)
            val bytes = baos.toByteArray()
            PendingImage(
                bytes = bytes,
                filename = "photo-${System.currentTimeMillis()}.jpg",
                thumbDataUri = "data:image/jpeg;base64," +
                    Base64.encodeToString(bytes, Base64.NO_WRAP),
                preview = bmp)
        } catch (e: Exception) {
            android.util.Log.e("SyncApi", "image decode failed", e)
            null
        }
    }

@Composable
fun ChatTab(config: Config, onDisconnect: () -> Unit) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var messages by remember {
        mutableStateOf(config.loadChat().ifEmpty {
            listOf(ChatMsg("perihel",
                "Send me a thought — I'll remember it, understand it, and remind you when it matters."))
        })
    }
    var input by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var pending by remember { mutableStateOf<PendingImage?>(null) }
    var attachNote by remember { mutableStateOf("") }

    val pickImage = rememberLauncherForActivityResult(
        ActivityResultContracts.GetContent()) { uri ->
        if (uri != null) {
            attachNote = "preparing image…"
            scope.launch {
                pending = prepareImage(ctx, uri)
                attachNote = if (pending != null) "" else "couldn't load that image"
            }
        }
    }

    fun append(msg: ChatMsg) {
        messages = messages + msg
        config.saveChat(messages)
    }

    // flush anything queued while the server was down
    LaunchedEffect(Unit) {
        val queue = config.loadPending()
        if (queue.isNotEmpty()) {
            scope.launch {
                val flushed = withContext(Dispatchers.IO) {
                    try { SyncApi.flushPending(config.url, config.token, queue); true }
                    catch (e: Exception) {
                        android.util.Log.e("SyncApi", "pending flush failed", e); false
                    }
                }
                if (flushed) {
                    config.savePending(emptyList())
                    append(ChatMsg("perihel",
                        "synced ${queue.size} offline thought${if (queue.size == 1) "" else "s"} to the logbook"))
                }
            }
        }
    }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically) {
            Text("Perihel", fontSize = 20.sp, color = Accent, modifier = Modifier.weight(1f))
            TextButton(onClick = onDisconnect) { Text("server", fontSize = 11.sp, color = Dim) }
        }

        LazyColumn(Modifier.weight(1f).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(messages) { msg ->
                val mine = msg.who == "you"
                Box(Modifier.fillMaxWidth()) {
                    Column(
                        Modifier
                            .align(if (mine) Alignment.CenterEnd else Alignment.CenterStart)
                            .background(if (mine) Surface1 else Color.Transparent,
                                MaterialTheme.shapes.medium)
                            .padding(horizontal = if (mine) 14.dp else 0.dp,
                                vertical = if (mine) 10.dp else 2.dp)
                            .widthIn(max = 320.dp),
                    ) {
                        msg.image?.let {
                            Image(it.asImageBitmap(), "photo",
                                Modifier.fillMaxWidth().heightIn(max = 220.dp)
                                    .clip(RoundedCornerShape(10.dp)),
                                contentScale = ContentScale.Crop)
                            Spacer(Modifier.height(4.dp))
                        }
                        Text(msg.text, fontSize = 15.sp,
                            color = if (mine) Color.Unspecified else Color(0xFFD6D6E0),
                            lineHeight = 20.sp)
                        if (msg.extra.isNotEmpty()) {
                            Text(msg.extra, fontSize = 11.sp, color = Dim,
                                modifier = Modifier.padding(top = 4.dp))
                        }
                    }
                }
            }
            if (busy) item {
                Text("thinking…", fontSize = 13.sp, color = Dim,
                    modifier = Modifier.padding(start = 4.dp))
            }
        }

        pending?.let { p ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically) {
                Image(p.preview.asImageBitmap(), "attached photo",
                    Modifier.height(72.dp).clip(RoundedCornerShape(10.dp)),
                    contentScale = ContentScale.Crop)
                Spacer(Modifier.width(8.dp))
                Text(attachNote.ifEmpty { "image attached" }, fontSize = 11.sp,
                    color = Dim, modifier = Modifier.weight(1f))
                TextButton(onClick = { pending = null }) { Text("✕", fontSize = 12.sp, color = Dim) }
            }
        }

        Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = { if (!busy) pickImage.launch("image/*") },
                enabled = !busy) {
                Text("📎", fontSize = 18.sp)
            }
            OutlinedTextField(input, { input = it },
                placeholder = { Text("capture a thought…", fontSize = 14.sp) },
                modifier = Modifier.weight(1f), maxLines = 4, enabled = !busy)
            Spacer(Modifier.width(8.dp))
            Button(onClick = {
                val text = input.trim()
                val img = pending
                if ((text.isNotEmpty() || img != null) && !busy) {
                    input = ""
                    pending = null
                    busy = true
                    append(ChatMsg("you", text.ifEmpty { "(photo)" }, image = img?.preview))
                    scope.launch {
                        var result: SyncApi.ChatResult? = null
                        var offlineReply: Pair<String, String>? = null
                        var offlineQueued = false
                        try {
                            result = withContext(Dispatchers.IO) {
                                var media: org.json.JSONObject? = null
                                if (img != null) {
                                    val up = SyncApi.uploadMedia(
                                        config.url, config.token, img.bytes, img.filename)
                                    media = org.json.JSONObject()
                                        .put("id", up.getString("id"))
                                        .put("size", up.optInt("size", img.bytes.size))
                                        .put("kind", up.optString("kind", "image"))
                                        .put("thumb", img.thumbDataUri)
                                }
                                SyncApi.chat(config.url, config.token, text, true, media)
                            }
                        } catch (e: Exception) {
                            android.util.Log.e("SyncApi", "chat failed — falling back to direct NIM", e)
                        }
                        if (result == null) {
                            // Server unreachable: the thought is saved on the phone
                            // FIRST (never lost), then AI still answers via direct NIM.
                            offlineQueued = true
                            val uuid = java.util.UUID.randomUUID().toString()
                            val stamp = java.time.OffsetDateTime.now()
                                .format(java.time.format.DateTimeFormatter.ISO_OFFSET_DATE_TIME)
                            offlineReply = NimApi.analyze(BuildConfig.DEFAULT_NIM_KEY,
                                text, img?.thumbDataUri)
                            val queue = config.loadPending() + SyncApi.PendingSend(
                                uuid, text.ifEmpty { "(photo)" }, img?.thumbDataUri, stamp,
                                offlineReply?.first, offlineReply?.second)
                            config.savePending(queue)
                        }
                        busy = false
                        val reply = when {
                            result?.aiState == "done" -> result.aiReply
                            result != null -> result.aiReply.ifEmpty { "saved to logbook (AI state: ${result.aiState})" }
                            offlineReply != null -> "offline — thought is safe on the phone and will sync when the server is back\n\n${offlineReply.first}"
                            offlineQueued -> "offline — thought is safe on the phone and will sync when the server is back (offline AI unavailable)"
                            else -> "⚠ couldn't reach the server — the thought is queued on the phone and will sync later"
                        }
                        val extra = buildString {
                            if (offlineQueued) append("queued on phone")
                            val cat = result?.category ?: offlineReply?.second
                            if (!cat.isNullOrEmpty()) {
                                if (isNotEmpty()) append(" · ")
                                append("category: $cat")
                            }
                            result?.reminders?.forEach { (_, d) ->
                                if (isNotEmpty()) append(" · ")
                                append("⏰ ${formatTime(d)}")
                            }
                        }
                        append(ChatMsg("perihel", reply, extra))
                    }
                }
            }, enabled = !busy) { Text(if (busy) "…" else "➤") }
        }
    }
}

// ---------------- tab 2: logbook ----------------

@Composable
fun LogbookTab(config: Config) {
    val scope = rememberCoroutineScope()
    var captures by remember { mutableStateOf<List<SyncApi.Capture>>(emptyList()) }
    var status by remember { mutableStateOf("loading…") }
    var typeFilter by remember { mutableStateOf(setOf<String>()) }
    var catFilter by remember { mutableStateOf(setOf<String>()) }
    var showDone by remember { mutableStateOf(true) }

    fun refresh() {
        scope.launch {
            status = "syncing…"
            try {
                captures = withContext(Dispatchers.IO) { SyncApi.pullCaptures(config.url, config.token) }
                    .sortedByDescending { it.createdAt }
                status = "${captures.count { it.status == "pending" }} pending · ${captures.size} total"
            } catch (e: Exception) { status = e.message ?: "sync failed" }
        }
    }
    LaunchedEffect(Unit) { refresh() }

    Column(Modifier.fillMaxSize().padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(status, fontSize = 12.sp, color = Dim, modifier = Modifier.weight(1f))
            TextButton(onClick = { showDone = !showDone }) {
                Text(if (showDone) "hide done" else "show done", fontSize = 11.sp)
            }
            TextButton(onClick = { refresh() }) { Text("sync", fontSize = 11.sp) }
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            TYPES.forEach { t ->
                FilterChip(selected = t in typeFilter,
                    onClick = { typeFilter = if (t in typeFilter) typeFilter - t else typeFilter + t },
                    label = { Text(t, fontSize = 11.sp) },
                    colors = FilterChipDefaults.filterChipColors(
                        selectedContainerColor = Surface1, selectedLabelColor = Accent))
            }
        }
        Row(Modifier.fillMaxWidth().padding(top = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            CATEGORIES.forEach { c ->
                FilterChip(selected = c in catFilter,
                    onClick = { catFilter = if (c in catFilter) catFilter - c else catFilter + c },
                    label = { Text(c, fontSize = 11.sp) },
                    colors = FilterChipDefaults.filterChipColors(
                        selectedContainerColor = Surface1, selectedLabelColor = Accent))
            }
        }
        Spacer(Modifier.height(8.dp))

        val filtered = captures.filter { c ->
            (typeFilter.isEmpty() || c.contentType in typeFilter) &&
            (catFilter.isEmpty() || c.category in catFilter) &&
            (showDone || c.status == "pending")
        }

        LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            items(filtered, key = { it.uuid }) { cap ->
                LogbookRow(cap, onToggle = {
                    scope.launch {
                        try {
                            withContext(Dispatchers.IO) {
                                SyncApi.setStatus(config.url, config.token, cap,
                                    if (cap.status == "pending") "done" else "pending")
                            }
                            refresh()
                        } catch (e: Exception) { status = e.message ?: "update failed" }
                    }
                }, onDelete = {
                    scope.launch {
                        try {
                            withContext(Dispatchers.IO) { SyncApi.softDelete(config.url, config.token, cap.uuid) }
                            refresh()
                        } catch (e: Exception) { status = e.message ?: "delete failed" }
                    }
                }, onRetry = {
                    scope.launch {
                        try {
                            withContext(Dispatchers.IO) { SyncApi.retryAi(config.url, config.token, cap.uuid) }
                            status = "requeued for AI — check back in a few minutes"
                        } catch (e: Exception) { status = e.message ?: "retry failed" }
                    }
                })
            }
        }
    }
}

@Composable
fun LogbookRow(cap: SyncApi.Capture, onToggle: () -> Unit, onDelete: () -> Unit, onRetry: () -> Unit) {
    val done = cap.status == "done"
    Column(Modifier.fillMaxWidth()
        .background(Surface1, MaterialTheme.shapes.medium)
        .clickable { onToggle() }
        .padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(9.dp).background(when {
                done -> DoneGreen
                cap.aiState == "error" -> Warn
                else -> Accent
            }, CircleShape))
            Spacer(Modifier.width(8.dp))
            Text(if (cap.contentType == "image") "🖼️ ${cap.content}" else cap.content,
                fontSize = 14.sp, maxLines = 2, overflow = TextOverflow.Ellipsis,
                color = if (done) Dim else Color.Unspecified, modifier = Modifier.weight(1f))
            TextButton(onClick = onDelete) { Text("✕", fontSize = 12.sp, color = Dim) }
        }
        if (!cap.aiSummary.isNullOrEmpty()) {
            Text(cap.aiSummary, fontSize = 12.sp, color = Color(0xFFA9A9B8),
                maxLines = 3, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(start = 17.dp, top = 4.dp))
        }
        Row(Modifier.padding(start = 17.dp, top = 4.dp)) {
            cap.category?.let {
                Text("· $it", fontSize = 10.sp, color = Accent)
                Spacer(Modifier.width(8.dp))
            }
            Text("· ${cap.contentType}", fontSize = 10.sp, color = Dim)
            cap.dueAt?.let {
                Spacer(Modifier.width(8.dp)); Text("· ⏰ ${formatTime(it)}", fontSize = 10.sp, color = Warn)
            }
            if (cap.aiState == "error") {
                Spacer(Modifier.weight(1f))
                TextButton(onClick = onRetry) { Text("retry AI", fontSize = 10.sp, color = Warn) }
            }
        }
    }
}

// ---------------- tab 3: alerts ----------------

@Composable
fun AlertsTab(config: Config) {
    val scope = rememberCoroutineScope()
    var reminders by remember { mutableStateOf<List<SyncApi.Reminder>>(emptyList()) }
    var settings by remember { mutableStateOf(mapOf<String, String>()) }
    var status by remember { mutableStateOf("loading…") }

    fun refresh() {
        scope.launch {
            status = "loading…"
            try {
                val (rems, sett) = withContext(Dispatchers.IO) {
                    SyncApi.pullReminders(config.url, config.token) to
                        SyncApi.pullSettings(config.url, config.token)
                }
                reminders = rems; settings = sett
                status = "updated now"
            } catch (e: Exception) { status = e.message ?: "load failed" }
        }
    }
    LaunchedEffect(Unit) { refresh() }

    Column(Modifier.fillMaxSize().padding(16.dp)) {
        Text("Alerts", fontSize = 20.sp, color = Accent)
        Text("nudges every ${settings["nudge_interval_hours"] ?: "?"}h · " +
            "active ${settings["active_start"] ?: "?"}–${settings["active_end"] ?: "?"} · " +
            "your explicit reminders fire any time",
            fontSize = 11.sp, color = Dim)
        Text("pushes arrive via Telegram (@perihelcloudbot) or ntfy; this tab shows what's scheduled",
            fontSize = 10.sp, color = Dim)
        Spacer(Modifier.height(12.dp))
        Row {
            Text(status, fontSize = 11.sp, color = Dim, modifier = Modifier.weight(1f))
            TextButton(onClick = { refresh() }) { Text("refresh", fontSize = 11.sp) }
        }
        Spacer(Modifier.height(8.dp))

        Text("Reminders", fontSize = 14.sp, color = Color(0xFFD6D6E0))
        Spacer(Modifier.height(6.dp))
        LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (reminders.isEmpty()) item {
                Text("no reminders yet — thoughts that mention a time create one automatically",
                    fontSize = 12.sp, color = Dim, modifier = Modifier.padding(vertical = 8.dp))
            }
            items(reminders, key = { it.id }) { rem ->
                Row(Modifier.fillMaxWidth()
                    .background(Surface1, MaterialTheme.shapes.medium).padding(12.dp),
                    verticalAlignment = Alignment.CenterVertically) {
                    Text(if (rem.status == "fired") "✓" else "⏰", fontSize = 15.sp,
                        color = if (rem.status == "fired") DoneGreen else Warn)
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text(rem.text, fontSize = 14.sp)
                        Text(formatTime(rem.dueAt) + if (rem.status == "fired") " · fired" else "",
                            fontSize = 11.sp, color = Dim)
                    }
                }
            }
        }
    }
}

private fun formatTime(rfc3339: String): String = try {
    DateTimeFormatter.ofPattern("MMM d HH:mm")
        .withZone(ZoneId.of("Asia/Hong_Kong"))
        .format(Instant.parse(rfc3339))
} catch (_: Exception) { rfc3339.take(16) }
