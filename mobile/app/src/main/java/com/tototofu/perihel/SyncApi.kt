package com.tototofu.perihel

import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID

/**
 * Client for the Perihel sync server — stdlib only.
 * Chat goes through POST /v1/chat which always saves the capture first,
 * then runs AI server-side (NVIDIA NIM); "ai":false skips processing.
 */
object SyncApi {
    private const val TAG = "SyncApi"

    data class Capture(
        val uuid: String,
        val content: String,
        val contentType: String,
        val status: String,
        val createdAt: String,
        val scheduledAt: String?,
        val aiSummary: String?,
        val category: String?,
        val aiState: String?,
        val dueAt: String?,
        val updatedAt: String,
        val deleted: Boolean,
    )

    data class ChatResult(
        val capture: Capture,
        val aiState: String,      // done | error | retrying | off
        val aiReply: String,
        val category: String?,
        val reminders: List<Pair<String, String>>,
    )

    data class Reminder(
        val id: Long,
        val text: String,
        val dueAt: String,
        val status: String,
    )

    class AuthError : Exception("server rejected the token")

    fun health(url: String, token: String) {
        request(url.trimEnd('/') + "/v1/health", token, "GET")
    }

    fun pullCaptures(url: String, token: String): List<Capture> {
        val body = request(url.trimEnd('/') + "/v1/captures", token, "GET")
            ?: throw Exception("empty response")
        val captures = JSONObject(String(body)).getJSONArray("captures")
        return (0 until captures.length())
            .map { parse(captures.getJSONObject(it)) }
            .filterNot { it.deleted }  // soft-deleted hidden on clients
    }

    /** Chat: save + AI in one call. `ai=false` just saves. */
    fun chat(url: String, token: String, content: String, ai: Boolean): ChatResult {
        val payload = JSONObject().put("content", content).put("ai", ai)
        val body = request(url.trimEnd('/') + "/v1/chat", token, "POST",
            payload.toString().toByteArray())
            ?: throw Exception("empty response")
        val v = JSONObject(String(body))
        val aiObj = v.optJSONObject("ai")
        val rems = ArrayList<Pair<String, String>>()
        if (aiObj?.has("reminders") == true) {
            val arr = aiObj.getJSONArray("reminders")
            for (i in 0 until arr.length()) {
                val r = arr.getJSONObject(i)
                rems.add(Pair(r.getString("text"), r.getString("due_at")))
            }
        }
        return ChatResult(
            capture = parse(v.getJSONObject("capture")),
            aiState = aiObj?.optString("state") ?: "off",
            aiReply = aiObj?.optString("reply") ?: "",
            category = aiObj?.takeIf { it.has("category") }?.optString("category"),
            reminders = rems,
        )
    }

    fun setStatus(url: String, token: String, capture: Capture, status: String): Capture {
        val now = nowRfc3339()
        val cap = JSONObject().apply {
            put("uuid", capture.uuid)
            put("content", capture.content)
            put("content_type", capture.contentType)
            put("status", status)
            put("created_at", capture.createdAt)
            capture.scheduledAt?.let { put("scheduled_at", it) }
            capture.category?.let { put("category", it) }
            put("updated_at", now)
        }
        postCaptures(url, token, cap)
        return parse(cap)
    }

    fun softDelete(url: String, token: String, uuid: String) {
        request(url.trimEnd('/') + "/v1/captures/delete", token, "POST",
            JSONObject().put("uuid", uuid).toString().toByteArray())
    }

    fun retryAi(url: String, token: String, uuid: String) {
        request(url.trimEnd('/') + "/v1/captures/retry", token, "POST",
            JSONObject().put("uuid", uuid).toString().toByteArray())
    }

    fun pullReminders(url: String, token: String): List<Reminder> {
        val body = request(url.trimEnd('/') + "/v1/reminders", token, "GET")
            ?: throw Exception("empty response")
        val arr = JSONObject(String(body)).getJSONArray("reminders")
        return (0 until arr.length()).map {
            val r = arr.getJSONObject(it)
            Reminder(r.getLong("id"), r.getString("text"), r.getString("due_at"),
                r.getString("status"))
        }
    }

    fun pullSettings(url: String, token: String): Map<String, String> {
        val body = request(url.trimEnd('/') + "/v1/settings", token, "GET")
            ?: throw Exception("empty response")
        val v = JSONObject(String(body))
        return v.keys().asSequence().associateWith { v.optString(it) }
    }

    private fun postCaptures(url: String, token: String, cap: JSONObject) {
        val payload = JSONObject().put("captures", JSONArray().put(cap))
        request(url.trimEnd('/') + "/v1/captures", token, "POST",
            payload.toString().toByteArray())
    }

    private fun parse(o: JSONObject) = Capture(
        uuid = o.getString("uuid"),
        content = o.optString("content", ""),
        contentType = o.optString("content_type", "text"),
        status = o.optString("status", "pending"),
        createdAt = o.optString("created_at", ""),
        scheduledAt = if (o.isNull("scheduled_at")) null else o.optString("scheduled_at"),
        aiSummary = if (o.isNull("ai_summary")) null else o.optString("ai_summary"),
        category = if (o.isNull("category")) null else o.optString("category"),
        aiState = if (o.isNull("ai_state")) null else o.optString("ai_state"),
        dueAt = if (o.isNull("due_at")) null else o.optString("due_at"),
        updatedAt = o.optString("updated_at", ""),
        deleted = o.optBoolean("deleted", false),
    )

    private fun request(urlStr: String, token: String, method: String,
                        body: ByteArray? = null): ByteArray? {
        val conn = URL(urlStr).openConnection() as HttpURLConnection
        conn.requestMethod = method
        conn.connectTimeout = 15_000
        conn.readTimeout = 120_000  // chat waits on the AI call
        conn.setRequestProperty("Authorization", "Bearer $token")
        if (body != null) {
            conn.doOutput = true
            conn.setRequestProperty("Content-Type", "application/json")
            conn.setFixedLengthStreamingMode(body.size)
            conn.outputStream.use { it.write(body) }
        }
        val code = conn.responseCode
        if (code == 401) throw AuthError()
        if (code !in 200..299) {
            val err = try { conn.errorStream?.bufferedReader()?.readText()?.take(200) } catch (_: Exception) { null }
            Log.w(TAG, "$method $urlStr -> $code $err")
            throw Exception("HTTP $code from server")
        }
        return if (method == "POST") null else conn.inputStream.readBytes()
    }

    private fun nowRfc3339(): String =
        java.time.OffsetDateTime.now().format(java.time.format.DateTimeFormatter.ISO_OFFSET_DATE_TIME)
}
