package com.tototofu.perihel

import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * Direct NVIDIA NIM client — the offline path. When the sync server (VM) is
 * unreachable, the phone still owes the user an AI reply: this calls NIM
 * straight from the device with the same strict-JSON prompt the server's
 * fallback uses (no tools, no memory — the thought is queued for the server,
 * which will run its full pipeline on flush if needed).
 *
 * Vision turns follow the server's hard-won rules: NO system role, a single
 * user turn with text+image parts, 240s timeout (vision queues on free tier).
 */
object NimApi {
    private const val TAG = "NimApi"
    private const val URL = "https://integrate.api.nvidia.com/v1/chat/completions"
    private const val MODEL = "meta/llama-3.2-11b-vision-instruct"

    private val JSON_INSTRUCTION =
        "Reply with ONLY minified JSON, no prose: " +
        "{\"summary\": str, \"category\": one of [\"idea\",\"todo\",\"link\",\"note\",\"question\",\"person\",\"event\"]}"

    /** Returns (summary, category) or null on any failure. */
    fun analyze(key: String, text: String, thumbDataUri: String?): Pair<String, String>? {
        if (key.isBlank()) return null
        return try {
            val message = JSONObject()
            if (thumbDataUri != null) {
                message.put("role", "user")
                message.put("content", JSONArray()
                    .put(JSONObject()
                        .put("type", "text")
                        .put("text", "$JSON_INSTRUCTION\n\nUser thought: ${text.ifEmpty { "(photo)" }}"))
                    .put(JSONObject()
                        .put("type", "image_url")
                        .put("image_url", JSONObject().put("url", thumbDataUri))))
            } else {
                message.put("role", "user")
                message.put("content", "$JSON_INSTRUCTION\n\nUser thought: $text")
            }
            val body = JSONObject()
                .put("model", MODEL)
                .put("messages", JSONArray().put(message))
                .put("max_tokens", 450)
                .put("temperature", 0.3)
                .put("stream", false)
            val conn = URL(URL).openConnection() as HttpURLConnection
            conn.requestMethod = "POST"
            conn.connectTimeout = 15_000
            conn.readTimeout = 240_000  // vision turns queue long on the free tier
            conn.setRequestProperty("Authorization", "Bearer $key")
            conn.setRequestProperty("Content-Type", "application/json")
            conn.doOutput = true
            val payload = body.toString().toByteArray()
            conn.setFixedLengthStreamingMode(payload.size)
            conn.outputStream.use { it.write(payload) }
            val code = conn.responseCode
            if (code !in 200..299) {
                Log.w(TAG, "NIM -> $code")
                return null
            }
            val out = JSONObject(String(conn.inputStream.readBytes()))
            val content = out.getJSONArray("choices").getJSONObject(0)
                .getJSONObject("message").getString("content").trim()
            val start = content.indexOf('{')
            val end = content.lastIndexOf('}')
            if (start < 0 || end <= start) return null
            val parsed = JSONObject(content.substring(start, end + 1))
            val summary = parsed.optString("summary", "").takeIf { it.isNotBlank() } ?: return null
            var category = parsed.optString("category", "note")
            if (category !in listOf("idea", "todo", "link", "note", "question", "person", "event")) {
                category = "note"
            }
            summary to category
        } catch (e: Exception) {
            Log.e(TAG, "NIM direct call failed", e)
            null
        }
    }
}
