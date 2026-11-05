package ru.catpaws.nebulaclient

import android.content.Context
import org.json.JSONObject
import ru.catpaws.nebula.nebulacore.Nebulacore

/** Вход в сеть, продление сертификата, список участников. Одна операция за раз. */
object Repo {
    private val lock = Any()

    fun validName(s: String): Boolean =
        s.length in 1..63 && s[0].code < 128 && s[0].isLetterOrDigit() &&
            s.all { it.code < 128 && (it.isLetterOrDigit() || it in "._-") }

    private fun now() = System.currentTimeMillis() / 1000

    /** Новые панели отдают servers, старые только server. */
    private fun dnsServers(j: JSONObject?): List<String> {
        if (j == null) return emptyList()
        val all = Device.strings(j.optJSONArray("servers"))
        return all.ifEmpty { listOfNotNull(j.optString("server").takeIf { it.isNotEmpty() }) }
    }

    private fun payloadToDevice(p: JSONObject, old: Device?, privateKey: String, server: String, token: String): Device {
        val net = p.getJSONObject("network")
        val cidr = net.getString("cidr")
        return Device(
            server = server, token = token, name = p.getString("name"), ip = p.getString("ip"),
            prefix = if (p.has("prefix")) p.getInt("prefix") else cidr.substringAfter('/').toInt(),
            cidr = cidr, slug = net.getString("slug"), title = net.optString("title", net.getString("slug")),
            privateKey = privateKey, cert = p.getString("cert"), ca = p.getString("ca"),
            panelConfig = p.getString("config"), notAfter = p.getLong("not_after"),
            lastSync = now(), wantUp = old?.wantUp ?: true,
            peers = Device.parsePeers(p.optJSONArray("peers")), peersTs = now(),
            dns = dnsServers(p.optJSONObject("dns")), dnsDomains = Device.strings(p.optJSONObject("dns")?.optJSONArray("domains")),
            dnsOn = old?.dnsOn ?: true,
        )
    }

    fun enroll(c: Context, serverRaw: String, name: String, invite: String, net: String): Device = synchronized(lock) {
        if (Store.load(c) != null) {
            throw PanelError(0, tr("Устройство уже в сети. Сначала выйдите из неё", "This device is already in a network. Leave it first"))
        }
        val server = Panel.checkServer(serverRaw)
        val n = name.trim()
        if (!validName(n)) {
            throw PanelError(
                0,
                tr(
                    "Имя устройства: латиница, цифры, точка, дефис, подчёркивание, до 63 символов",
                    "Device name: Latin letters, digits, dot, dash, underscore, up to 63 characters",
                ),
            )
        }
        val kp = Nebulacore.generateKeypair()
        val body = JSONObject()
            .put("invite", invite.trim())
            .put("network", if (net.isBlank()) JSONObject.NULL else net.trim())
            .put("name", n)
            .put("public_key", kp.public)
            .put("conf_dir", CONF_DIR)
            .put("client", CLIENT_VERSION)
        val p = Panel.post(server, "/v1/enroll", null, body)
        val token = p.optString("token").ifEmpty {
            throw PanelError(0, tr("панель не выдала токен устройства", "the panel did not issue a device token"))
        }
        val d = payloadToDevice(p, null, kp.private, server, token)
        Store.save(c, d)
        d
    }

    /** true, если сертификат или конфиг поменялись и Nebula надо перезапустить. */
    fun sync(c: Context): Boolean = synchronized(lock) {
        val d = Store.load(c) ?: return false
        val body = JSONObject().put("conf_dir", CONF_DIR).put("client", CLIENT_VERSION)
        val p = Panel.post(d.server, "/v1/sync", d.token, body)
        val nd = payloadToDevice(p, d, d.privateKey, d.server, d.token)
        val changed = nd.cert != d.cert || nd.ca != d.ca || nd.dns != d.dns ||
            ConfigPrep.prepare(nd.panelConfig, nd) != ConfigPrep.prepare(d.panelConfig, d)
        Store.save(c, nd)
        changed
    }

    fun peers(c: Context): List<Peer> = synchronized(lock) {
        val d = Store.load(c) ?: return emptyList()
        runCatching {
            val j = Panel.get(d.server, "/v1/peers", d.token)
            val list = Device.parsePeers(j.optJSONArray("peers"))
            Store.save(c, d.copy(peers = list, peersTs = now()))
            list
        }.getOrDefault(d.peers)
    }

    fun setDnsOn(c: Context, on: Boolean) = synchronized(lock) {
        Store.load(c)?.let { Store.save(c, it.copy(dnsOn = on)) }
    }

    fun setWant(c: Context, up: Boolean) = synchronized(lock) {
        Store.load(c)?.let { Store.save(c, it.copy(wantUp = up)) }
    }

    /** Выйти из сети: отозвать сертификат на панели (если получится) и стереть ключ. Возвращает предупреждение, если панель не ответила. */
    fun leave(c: Context): String? = synchronized(lock) {
        val d = Store.load(c) ?: return null
        var warn: String? = null
        runCatching { Panel.post(d.server, "/v1/leave", d.token, JSONObject()) }.onFailure { warn = it.message }
        Store.clear(c)
        warn
    }
}
