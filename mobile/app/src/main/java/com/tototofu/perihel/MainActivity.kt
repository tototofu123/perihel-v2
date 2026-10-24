package com.tototofu.perihel

import android.content.Context
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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
    var aiEnabled: Boolean
        get() = prefs.getBoolean("ai_enabled", true)  // default AI ON
        set(v) = prefs.edit().putBoolean("ai_enabled", v).apply()
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

data class ChatMsg(val who: String, val text: String, val extra: String = "")

@Composable
fun ChatTab(config: Config, onDisconnect: () -> Unit) {
    val scope = rememberCoroutineScope()
    var messages by remember { mutableStateOf(listOf(ChatMsg("perihel",
        "Send me a thought — I'll remember it, understand it, and remind you when it matters."))) }
    var input by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically) {
            Text("Perihel", fontSize = 20.sp, color = Accent, modifier = Modifier.weight(1f))
            Text("AI", fontSize = 12.sp, color = if (config.aiEnabled) DoneGreen else Dim)
            Switch(
                checked = config.aiEnabled,
                onCheckedChange = { config.aiEnabled = it },
                modifier = Modifier.padding(start = 6.dp).height(28.dp),
                colors = SwitchDefaults.colors(checkedTrackColor = Accent),
            )
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

        Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(input, { input = it },
                placeholder = { Text("capture a thought…", fontSize = 14.sp) },
                modifier = Modifier.weight(1f), maxLines = 4, enabled = !busy)
            Spacer(Modifier.width(8.dp))
            Button(onClick = {
                val text = input.trim()
                if (text.isNotEmpty() && !busy) {
                    input = ""
                    busy = true
                    messages = messages + ChatMsg("you", text)
                    scope.launch {
                        var result: SyncApi.ChatResult? = null
                        var error: String? = null
                        try {
                            result = withContext(Dispatchers.IO) {
                                SyncApi.chat(config.url, config.token, text, config.aiEnabled)
                            }
                        } catch (e: Exception) {
                            android.util.Log.e("SyncApi", "chat failed", e)
                            error = e.message ?: e.toString()
                        }
                        busy = false
                        val reply = when {
                            error != null -> "⚠ couldn't reach the server ($error) — the thought was NOT lost: tap sync in Logbook after reconnecting"
                            result!!.aiState == "off" -> "saved to logbook (AI off)"
                            result.aiState == "done" -> result.aiReply
                            result.aiState == "retrying" -> result.aiReply
                            else -> result.aiReply.ifEmpty { "saved to logbook (AI state: ${result.aiState})" }
                        }
                        val extra = buildString {
                            result?.let { r ->
                                if (!r.category.isNullOrEmpty()) append("category: ${r.category}")
                                r.reminders.forEach { (_, d) ->
                                    if (isNotEmpty()) append(" · ")
                                    append("⏰ ${formatTime(d)}")
                                }
                            }
                        }
                        messages = messages + ChatMsg("perihel", reply, extra)
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
            Text(cap.content, fontSize = 14.sp, maxLines = 2, overflow = TextOverflow.Ellipsis,
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
        Text("pushes arrive via the ntfy app; this tab shows what's scheduled",
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
