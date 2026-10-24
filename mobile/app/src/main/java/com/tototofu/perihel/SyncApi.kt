package com.tototofu.perihel

import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID
import javax.net.ssl.HttpsURLConnection

/**
 * Minimal client for the Perihel sync server — stdlib only, no dependencies.
 * Same protocol as the desktop app: bearer token, last-write-wins on
 * `updated_at`. The server is the meeting point; the phone holds no DB and
 * re-pulls full state each refresh (personal scale, dozens of captures).
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
        val updatedAt: String,
    )

    class AuthError : Exception("server rejected the token")

    fun health(url: String, token: String) {
        request(url.trimEnd('/') + "/v1/health", token, "GET")
    }

    fun pullCaptures(url: String, token: String): List<Capture> {
        val body = request(url.trimEnd('/') + "/v1/captures", token, "GET")
            ?: throw Exception("empty response")
        val captures = JSONObject(String(body)).getJSONArray("captures")
        return (0 until captures.length()).map { parse(captures.getJSONObject(it)) }
    }

    /** Push one new capture. */
    fun addCapture(url: String, token: String, content: String): Capture {
        val now = nowRfc3339()
        val cap = JSONObject().apply {
            put("uuid", UUID.randomUUID().toString())
            put("content", content)
            put("content_type", "text")
            put("status", "pending")
            put("created_at", now)
            put("updated_at", now)
        }
        postCaptures(url, token, cap)
        return parse(cap)
    }

    /** Update status (e.g. mark done) — bumps updated_at so it wins the merge. */
    fun setStatus(url: String, token: String, capture: Capture, status: String): Capture {
        val now = nowRfc3339()
        val cap = JSONObject().apply {
            put("uuid", capture.uuid)
            put("content", capture.content)
            put("content_type", capture.contentType)
            put("status", status)
            put("created_at", capture.createdAt)
            capture.scheduledAt?.let { put("scheduled_at", it) }
            put("updated_at", now)
        }
        postCaptures(url, token, cap)
        return parse(cap)
    }

    private fun postCaptures(url: String, token: String, cap: JSONObject) {
        val payload = JSONObject().put("captures", JSONArray().put(cap))
        request(url.trimEnd('/') + "/v1/captures", token, "POST", payload.toString().toByteArray())
    }

    private fun parse(o: JSONObject) = Capture(
        uuid = o.getString("uuid"),
        content = o.optString("content", ""),
        contentType = o.optString("content_type", "text"),
        status = o.optString("status", "pending"),
        createdAt = o.optString("created_at", ""),
        scheduledAt = if (o.isNull("scheduled_at")) null else o.optString("scheduled_at"),
        updatedAt = o.optString("updated_at", ""),
    )

    /** Executes the request; returns the response body for GETs, null for POSTs. */
    private fun request(urlStr: String, token: String, method: String, body: ByteArray? = null): ByteArray? {
        val conn = URL(urlStr).openConnection() as HttpURLConnection
        conn.requestMethod = method
        conn.connectTimeout = 10_000
        conn.readTimeout = 30_000
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
