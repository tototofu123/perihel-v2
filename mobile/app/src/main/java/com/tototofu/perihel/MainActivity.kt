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
import kotlinx.coroutines.launch
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
private val Dim = Color(0xFF8A8A96)
private val Surface0 = Color(0xFF141419)
private val Surface1 = Color(0xFF1C1C23)

@Composable
fun PerihelTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(primary = Accent, background = Surface0, surface = Surface0),
        content = content,
    )
}

/** Server URL + token, persisted in plain SharedPreferences. */
class Config(ctx: Context) {
    private val prefs = ctx.getSharedPreferences("perihel", Context.MODE_PRIVATE)
    var url: String
        get() = prefs.getString("url", BuildConfig.DEFAULT_URL) ?: ""
        set(v) = prefs.edit().putString("url", v).apply()
    var token: String
        get() = prefs.getString("token", BuildConfig.DEFAULT_TOKEN) ?: ""
        set(v) = prefs.edit().putString("token", v).apply()
    val isConfigured: Boolean get() = url.isNotBlank() && token.isNotBlank()
}

@Composable
fun PerihelApp() {
    val ctx = LocalContext.current
    val config = remember { Config(ctx) }
    var screen by remember { mutableStateOf(if (config.isConfigured) "list" else "setup") }
    if (screen == "setup") {
        SetupScreen(config) { screen = "list" }
    } else {
        CapturesScreen(config, onDisconnect = { screen = "setup" })
    }
}

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
            placeholder = { Text("https://1-2-3-4.sslip.io") },
            singleLine = true, modifier = Modifier.fillMaxWidth())
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(token, { token = it }, label = { Text("Token") },
            singleLine = true, modifier = Modifier.fillMaxWidth())
        Spacer(Modifier.height(16.dp))
        if (message.isNotEmpty()) Text(message, fontSize = 12.sp, color = MaterialTheme.colorScheme.error)
        Button(
            onClick = {
                scope.launch {
                    message = "checking…"
                    try {
                        SyncApi.health(url, token)
                        config.url = url.trim()
                        config.token = token.trim()
                        onDone()
                    } catch (e: Exception) {
                        message = e.message ?: "connection failed"
                    }
                }
            },
            modifier = Modifier.fillMaxWidth(),
        ) { Text("Connect") }
    }
}

@Composable
fun CapturesScreen(config: Config, onDisconnect: () -> Unit) {
    val scope = rememberCoroutineScope()
    var captures by remember { mutableStateOf<List<SyncApi.Capture>>(emptyList()) }
    var status by remember { mutableStateOf("loading…") }
    var newCapture by remember { mutableStateOf("") }

    fun refresh() {
        scope.launch {
            status = "syncing…"
            try {
                captures = SyncApi.pullCaptures(config.url, config.token)
                    .sortedByDescending { it.createdAt }
                status = "${captures.count { it.status == "pending" }} pending"
            } catch (e: SyncApi.AuthError) {
                status = "token rejected — reconnect"
                onDisconnect()
            } catch (e: Exception) {
                status = e.message ?: "sync failed"
            }
        }
    }
    LaunchedEffect(Unit) { refresh() }

    Column(Modifier.fillMaxSize().padding(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Perihel", fontSize = 22.sp, color = Accent, modifier = Modifier.weight(1f))
            TextButton(onClick = { onDisconnect() }) { Text("server", fontSize = 11.sp, color = Dim) }
            TextButton(onClick = { refresh() }) { Text("sync", fontSize = 11.sp) }
        }
        Text(status, fontSize = 12.sp, color = Dim)
        Spacer(Modifier.height(8.dp))

        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(newCapture, { newCapture = it },
                placeholder = { Text("capture a thought…", fontSize = 13.sp) },
                modifier = Modifier.weight(1f), singleLine = true)
            Spacer(Modifier.width(8.dp))
            Button(onClick = {
                if (newCapture.isNotBlank()) {
                    scope.launch {
                        try {
                            SyncApi.addCapture(config.url, config.token, newCapture.trim())
                            newCapture = ""
                            refresh()
                        } catch (e: Exception) { status = e.message ?: "add failed" }
                    }
                }
            }) { Text("+") }
        }
        Spacer(Modifier.height(12.dp))

        LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            items(captures, key = { it.uuid }) { cap ->
                CaptureRow(cap) {
                    scope.launch {
                        try {
                            SyncApi.setStatus(config.url, config.token, cap,
                                if (cap.status == "pending") "done" else "pending")
                            refresh()
                        } catch (e: Exception) { status = e.message ?: "update failed" }
                    }
                }
            }
        }
    }
}

@Composable
fun CaptureRow(cap: SyncApi.Capture, onToggle: () -> Unit) {
    val done = cap.status == "done"
    Row(
        Modifier.fillMaxWidth()
            .background(Surface1, MaterialTheme.shapes.medium)
            .clickable { onToggle() }
            .padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(10.dp).background(if (done) DoneGreen else Accent, CircleShape))
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(cap.content, fontSize = 14.sp, maxLines = 3, overflow = TextOverflow.Ellipsis,
                color = if (done) Dim else Color.Unspecified)
            cap.scheduledAt?.let {
                Text("resurfaces ${formatTime(it)}", fontSize = 11.sp, color = Dim)
            }
        }
    }
}

private fun formatTime(rfc3339: String): String = try {
    DateTimeFormatter.ofPattern("MMM d HH:mm")
        .withZone(ZoneId.systemDefault())
        .format(Instant.parse(rfc3339))
} catch (_: Exception) { rfc3339.take(16) }
