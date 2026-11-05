package ru.catpaws.nebulaclient

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

enum class UpdateMode { AUTO, NOTIFY, OFF }

/** Найденная версия новее установленной. */
data class Release(val version: String, val page: String, val apkUrl: String, val sha256: String)

sealed class UpdState {
    data object Idle : UpdState()
    data object Checking : UpdState()
    data class Available(val release: Release) : UpdState()
    data class Downloading(val release: Release, val percent: Int) : UpdState()
    data class Installing(val release: Release) : UpdState()
    data object UpToDate : UpdState()
    data class Error(val message: String) : UpdState()
}

/**
 * Обновления с GitHub. Лента релизов та же, что у десктопа, но берём только теги android-v*.
 * APK проверяется по SHA-256 из релиза (файл SHA256SUMS.txt или строка в описании), подпись сверяет сам Android:
 * чужим ключом обновление не встанет.
 */
object Updater {
    private const val PREFS = "update"
    const val CHANNEL = "updates"
    const val ACTION_RESULT = "ru.catpaws.nebulaclient.INSTALL_RESULT"
    private val VERSION = Regex("""(\d+)\.(\d+)\.(\d+)-android(\d+)""")

    private val _state = MutableStateFlow<UpdState>(UpdState.Idle)
    val state: StateFlow<UpdState> = _state

    fun set(s: UpdState) {
        _state.value = s
    }

    // ------------------------------------------------------------ настройки

    private fun prefs(c: Context) = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun mode(c: Context): UpdateMode = runCatching { UpdateMode.valueOf(prefs(c).getString("mode", "AUTO")!!) }.getOrDefault(UpdateMode.AUTO)

    fun setMode(c: Context, m: UpdateMode) = prefs(c).edit().putString("mode", m.name).apply()

    /** Пока все релизы тестовые (pre-release), без этого обновлений не было бы вовсе. */
    fun prerelease(c: Context): Boolean = prefs(c).getBoolean("pre", true)

    fun setPrerelease(c: Context, v: Boolean) = prefs(c).edit().putBoolean("pre", v).apply()

    fun lastCheck(c: Context): Long = prefs(c).getLong("last", 0)

    fun installedVersion(c: Context): String = c.packageManager.getPackageInfo(c.packageName, 0).versionName ?: "0"

    // ------------------------------------------------------------ версии

    fun parse(v: String): List<Int>? = VERSION.find(v)?.groupValues?.drop(1)?.map { it.toInt() }

    fun isNewer(candidate: String, installed: String): Boolean {
        val a = parse(candidate) ?: return false
        val b = parse(installed) ?: return true
        for (i in a.indices) if (a[i] != b[i]) return a[i] > b[i]
        return false
    }

    // ------------------------------------------------------------ проверка

    private fun http(url: String, accept: String? = null): HttpURLConnection {
        val conn = URL(url).openConnection() as HttpURLConnection
        conn.connectTimeout = 15_000
        conn.readTimeout = 60_000
        conn.setRequestProperty("User-Agent", CLIENT_VERSION)
        if (accept != null) conn.setRequestProperty("Accept", accept)
        return conn
    }

    private fun getText(url: String): String {
        val conn = http(url, "application/json, text/plain")
        try {
            if (conn.responseCode !in 200..299) throw PanelError(conn.responseCode, "HTTP ${conn.responseCode}")
            return conn.inputStream.bufferedReader().use { it.readText() }
        } finally {
            conn.disconnect()
        }
    }

    /** Найти релиз новее установленной версии. null, если новее ничего нет. */
    fun check(c: Context): Release? {
        val feed = BuildConfig.UPDATE_FEED
        val list = JSONArray(getText(feed))
        val pre = prerelease(c)
        var best: Release? = null
        for (i in 0 until list.length()) {
            val r = list.getJSONObject(i)
            if (r.optBoolean("draft") || (!pre && r.optBoolean("prerelease"))) continue
            if (!r.optString("tag_name").startsWith("android-v")) continue
            val assets = r.optJSONArray("assets") ?: continue
            var apk: JSONObject? = null
            var sums: JSONObject? = null
            for (j in 0 until assets.length()) {
                val a = assets.getJSONObject(j)
                val n = a.optString("name")
                if (n.endsWith(".apk") && parse(n) != null) apk = a
                if (n.equals("SHA256SUMS.txt", true)) sums = a
            }
            val apkAsset = apk ?: continue
            val version = VERSION.find(apkAsset.getString("name"))!!.value
            if (!isNewer(version, installedVersion(c))) continue
            if (best != null && !isNewer(version, best.version)) continue
            val sha = sha256Of(apkAsset.getString("name"), sums, r.optString("body"))
                ?: continue // без суммы не ставим
            best = Release(version, r.optString("html_url"), apkAsset.getString("browser_download_url"), sha)
        }
        prefs(c).edit().putLong("last", System.currentTimeMillis() / 1000).apply()
        return best
    }

    private fun sha256Of(file: String, sums: JSONObject?, body: String): String? {
        if (sums != null) {
            val text = runCatching { getText(sums.getString("browser_download_url")) }.getOrNull()
            text?.lines()?.forEach { line ->
                val p = line.trim().split(Regex("\\s+"), limit = 2)
                if (p.size == 2 && p[1].trimStart('*') == file && p[0].length == 64) return p[0].lowercase()
            }
        }
        return Regex("""\b[0-9a-fA-F]{64}\b""").find(body)?.value?.lowercase()
    }

    // ------------------------------------------------------------ скачивание и установка

    fun download(c: Context, r: Release): File {
        val dir = File(c.cacheDir, "updates").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val out = File(dir, "NebulaClient-${r.version}.apk")
        val conn = http(r.apkUrl)
        try {
            if (conn.responseCode !in 200..299) throw PanelError(conn.responseCode, "HTTP ${conn.responseCode}")
            val total = conn.contentLengthLong
            val md = MessageDigest.getInstance("SHA-256")
            var done = 0L
            var last = -1
            conn.inputStream.use { input ->
                out.outputStream().use { o ->
                    val buf = ByteArray(64 * 1024)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        o.write(buf, 0, n)
                        md.update(buf, 0, n)
                        done += n
                        if (total > 0) {
                            val pct = (done * 100 / total).toInt()
                            if (pct != last) {
                                last = pct
                                set(UpdState.Downloading(r, pct))
                            }
                        }
                    }
                }
            }
            val got = md.digest().joinToString("") { "%02x".format(it) }
            if (got != r.sha256) {
                out.delete()
                throw PanelError(0, tr("контрольная сумма APK не совпала, установка остановлена", "APK checksum mismatch, installation stopped"))
            }
        } finally {
            conn.disconnect()
        }
        return out
    }

    fun canInstall(c: Context): Boolean = c.packageManager.canRequestPackageInstalls()

    /** Окно настроек «Установка неизвестных приложений» для нашего приложения. */
    fun unknownSourcesIntent(c: Context): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${c.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    fun install(c: Context, apk: File) {
        val pi = c.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        params.setAppPackageName(c.packageName)
        // Android 12+: если приложение ставилось нами же, обновление идёт без вопроса
        if (Build.VERSION.SDK_INT >= 31) params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        val id = pi.createSession(params)
        pi.openSession(id).use { s ->
            apk.inputStream().use { i ->
                s.openWrite("nebula.apk", 0, apk.length()).use { o ->
                    i.copyTo(o)
                    s.fsync(o)
                }
            }
            val intent = Intent(c, InstallReceiver::class.java).setAction(ACTION_RESULT).setPackage(c.packageName)
            // изменяемый: установщик дописывает в него статус
            val pend = PendingIntent.getBroadcast(c, id, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            s.commit(pend.intentSender)
        }
    }

    /** Скачать, проверить и поставить. Исключения превращаются в состояние Error. */
    fun downloadAndInstall(c: Context, r: Release) {
        try {
            set(UpdState.Downloading(r, 0))
            val f = download(c, r)
            set(UpdState.Installing(r))
            install(c, f)
        } catch (e: Exception) {
            set(UpdState.Error(e.message ?: e.javaClass.simpleName))
        }
    }

    // ------------------------------------------------------------ уведомления

    fun notify(c: Context, id: Int, title: String, text: String, tap: PendingIntent) {
        val nm = c.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, tr("Обновления", "Updates"), NotificationManager.IMPORTANCE_DEFAULT))
        val n = NotificationCompat.Builder(c, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(tap)
            .setAutoCancel(true)
            .build()
        nm.notify(id, n)
    }

    fun openAppIntent(c: Context): PendingIntent =
        PendingIntent.getActivity(c, 0, Intent(c, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
}
