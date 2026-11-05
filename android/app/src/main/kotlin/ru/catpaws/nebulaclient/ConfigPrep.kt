package ru.catpaws.nebulaclient

/** Приводит конфиг панели к виду, который понимает Nebula внутри приложения. */
object ConfigPrep {
    private fun block(key: String, pem: String): List<String> =
        listOf("  $key: |") + pem.trim().lines().map { "    ${it.trimEnd()}" }

    /**
     * Ключи и сертификаты кладём в сам конфиг (pki.* принимает PEM). Адаптер выдаёт VpnService, поэтому tun.dev не нужен.
     * sshd для «живой карты» панели на телефоне не поднимаем.
     */
    fun prepare(panelConfig: String, d: Device): String {
        val out = ArrayList<String>()
        var skipping = false
        for (line in panelConfig.lines()) {
            if (skipping) {
                if (line.isEmpty() || line.startsWith(" ")) continue
                skipping = false
            }
            when {
                line == "sshd:" -> skipping = true
                line.startsWith("  ca: ") -> out += block("ca", d.ca)
                line.startsWith("  cert: ") -> out += block("cert", d.cert)
                line.startsWith("  key: ") -> out += block("key", d.privateKey)
                line.startsWith("  dev: ") -> {}
                else -> out += line
            }
        }
        return out.joinToString("\n") + "\n"
    }

    /** Адрес сети из CIDR (10.0.10.0/24 -> 10.0.10.0) и длина префикса. */
    fun route(cidr: String): Pair<String, Int> {
        val (addr, len) = cidr.split("/")
        return addr to len.toInt()
    }
}
