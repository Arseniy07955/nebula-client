package ru.catpaws.nebulaclient

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Участник сети из списка панели. */
data class Peer(val name: String, val ip: String, val role: String, val online: Boolean)

/** Всё, что телефон знает о своей сети. Лежит в закрытой папке приложения, ключ никуда не уходит. */
data class Device(
    val server: String,
    val token: String,
    val name: String,
    val ip: String,
    val prefix: Int,
    val cidr: String,
    val slug: String,
    val title: String,
    val privateKey: String,
    val cert: String,
    val ca: String,
    /** Конфиг от панели как есть (с путями к файлам), в Nebula идёт его подготовленная версия. */
    val panelConfig: String,
    val notAfter: Long,
    val lastSync: Long,
    val wantUp: Boolean,
    val peers: List<Peer>,
    val peersTs: Long = 0,
    /** DNS сети от панели (например, 10.0.10.7) и зоны, которые он обслуживает (например, neb). */
    val dns: List<String> = emptyList(),
    val dnsDomains: List<String> = emptyList(),
    val dnsOn: Boolean = true,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("server", server); put("token", token); put("name", name); put("ip", ip); put("prefix", prefix)
        put("cidr", cidr); put("slug", slug); put("title", title); put("privateKey", privateKey)
        put("cert", cert); put("ca", ca); put("panelConfig", panelConfig); put("notAfter", notAfter)
        put("lastSync", lastSync); put("wantUp", wantUp); put("peersTs", peersTs)
        put("dns", JSONArray(dns)); put("dnsDomains", JSONArray(dnsDomains)); put("dnsOn", dnsOn)
        put("peers", JSONArray(peers.map {
            JSONObject().put("name", it.name).put("ip", it.ip).put("role", it.role).put("online", it.online)
        }))
    }

    companion object {
        fun fromJson(j: JSONObject) = Device(
            server = j.getString("server"), token = j.getString("token"), name = j.getString("name"),
            ip = j.getString("ip"), prefix = j.getInt("prefix"), cidr = j.getString("cidr"),
            slug = j.getString("slug"), title = j.getString("title"), privateKey = j.getString("privateKey"),
            cert = j.getString("cert"), ca = j.getString("ca"), panelConfig = j.getString("panelConfig"),
            notAfter = j.getLong("notAfter"), lastSync = j.optLong("lastSync"), wantUp = j.optBoolean("wantUp"),
            peers = parsePeers(j.optJSONArray("peers")), peersTs = j.optLong("peersTs"),
            dns = strings(j.optJSONArray("dns")), dnsDomains = strings(j.optJSONArray("dnsDomains")),
            dnsOn = j.optBoolean("dnsOn", true),
        )

        fun strings(a: JSONArray?): List<String> = (0 until (a?.length() ?: 0)).map { a!!.getString(it) }

        fun parsePeers(a: JSONArray?): List<Peer> = (0 until (a?.length() ?: 0)).map {
            val p = a!!.getJSONObject(it)
            Peer(p.optString("name"), p.optString("ip"), p.optString("role"), p.optBoolean("online"))
        }
    }
}

object Store {
    private fun file(c: Context) = File(c.filesDir, "device.json")

    @Synchronized
    fun load(c: Context): Device? = runCatching { Device.fromJson(JSONObject(file(c).readText())) }.getOrNull()

    @Synchronized
    fun save(c: Context, d: Device) {
        val tmp = File(c.filesDir, "device.json.tmp")
        tmp.writeText(d.toJson().toString())
        tmp.renameTo(file(c))
    }

    @Synchronized
    fun clear(c: Context) {
        file(c).delete()
    }
}
