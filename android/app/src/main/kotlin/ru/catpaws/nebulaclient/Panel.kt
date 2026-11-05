package ru.catpaws.nebulaclient

import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.Locale

const val CLIENT_VERSION = "nebula-client-android/0.5.1"
const val DEFAULT_SERVER = "https://svyaz.msk1.catpaws.ru"

/** Каталог, под который панель пишет пути в конфиге; потом мы заменяем их на содержимое файлов. */
const val CONF_DIR = "/data/nebula"

class PanelError(val code: Int, message: String) : Exception(message)

fun isRu(): Boolean = Locale.getDefault().language == "ru"

fun tr(ru: String, en: String): String = if (isRu()) ru else en

object Panel {
    fun checkServer(s: String): String {
        val t = s.trim().trimEnd('/')
        if (!t.startsWith("https://")) {
            throw PanelError(0, tr("Адрес панели должен начинаться с https://", "The panel address must start with https://"))
        }
        return t
    }

    fun post(server: String, path: String, token: String?, body: JSONObject): JSONObject = call("POST", server, path, token, body)

    fun get(server: String, path: String, token: String?): JSONObject = call("GET", server, path, token, null)

    private fun call(method: String, server: String, path: String, token: String?, body: JSONObject?): JSONObject {
        val conn = URL(server + path).openConnection() as HttpURLConnection
        try {
            conn.requestMethod = method
            conn.connectTimeout = 10_000
            conn.readTimeout = 60_000
            conn.setRequestProperty("User-Agent", CLIENT_VERSION)
            conn.setRequestProperty("Accept", "application/json")
            conn.setRequestProperty("Accept-Language", if (isRu()) "ru" else "en")
            if (token != null) conn.setRequestProperty("Authorization", "Bearer $token")
            if (body != null) {
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                conn.outputStream.use { it.write(body.toString().toByteArray()) }
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader()?.use { it.readText() } ?: ""
            val json = runCatching { JSONObject(text) }.getOrNull()
            if (code !in 200..299) {
                val msg = json?.optString("error")?.takeIf { it.isNotEmpty() }
                    ?: tr("панель ответила $code", "the panel answered $code")
                throw PanelError(code, msg)
            }
            return json ?: throw PanelError(code, tr("непонятный ответ панели", "unexpected panel answer"))
        } catch (e: PanelError) {
            throw e
        } catch (e: IOException) {
            throw PanelError(0, tr("нет связи с панелью: ", "cannot reach the panel: ") + (e.message ?: e.javaClass.simpleName))
        } finally {
            conn.disconnect()
        }
    }
}
