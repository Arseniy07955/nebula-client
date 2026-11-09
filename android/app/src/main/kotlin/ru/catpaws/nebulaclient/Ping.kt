package ru.catpaws.nebulaclient

import android.system.Os
import android.system.OsConstants
import android.system.StructPollfd
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import java.net.InetAddress

/**
 * Пинг участников по туннелю. Приложение не исключено из своего VPN, поэтому пакеты к адресам сети идут через Nebula.
 * ICMP-сокет без root (ping socket, SOCK_DGRAM + IPPROTO_ICMP): заголовок и контрольную сумму ядро дописывает само.
 * Если прошивка его не даёт, запасной путь через /system/bin/ping.
 */
object Ping {
    private const val TIMEOUT_MS = 1000

    /** RTT в мс по каждому адресу, null если ответа нет. Все адреса пингуются одновременно. */
    suspend fun all(ips: List<String>): Map<String, Float?> = coroutineScope {
        ips.map { ip -> async(Dispatchers.IO) { ip to rtt(ip) } }.awaitAll().toMap()
    }

    suspend fun rtt(ip: String): Float? = withContext(Dispatchers.IO) {
        runCatching { socketPing(ip) }.getOrElse { runCatching { binaryPing(ip) }.getOrNull() }
    }

    private fun socketPing(ip: String): Float? {
        val fd = Os.socket(OsConstants.AF_INET, OsConstants.SOCK_DGRAM, OsConstants.IPPROTO_ICMP)
        try {
            // эхо-запрос: тип 8, код 0; идентификатор подставит ядро, номер 1, 8 байт данных
            val pkt = byteArrayOf(8, 0, 0, 0, 0, 0, 0, 1) + ByteArray(8)
            val t0 = System.nanoTime()
            Os.sendto(fd, pkt, 0, pkt.size, 0, InetAddress.getByName(ip), 0)
            val buf = ByteArray(128)
            while (true) {
                val left = TIMEOUT_MS - ((System.nanoTime() - t0) / 1_000_000).toInt()
                if (left <= 0) return null
                val pfd = StructPollfd().apply { this.fd = fd; events = OsConstants.POLLIN.toShort() }
                if (Os.poll(arrayOf(pfd), left) <= 0) return null
                val n = Os.recvfrom(fd, buf, 0, buf.size, 0, null)
                // ответ (тип 0) на наш номер; заголовка IP в ping-сокете нет
                if (n >= 8 && buf[0].toInt() == 0 && buf[7].toInt() == 1) return (System.nanoTime() - t0) / 1e6f
            }
        } finally {
            Os.close(fd)
        }
    }

    private val TIME = Regex("time=([0-9.]+)")

    private fun binaryPing(ip: String): Float? {
        val p = ProcessBuilder("/system/bin/ping", "-c", "1", "-W", "1", ip).redirectErrorStream(true).start()
        val out = p.inputStream.bufferedReader().readText()
        p.waitFor()
        return TIME.find(out)?.groupValues?.get(1)?.toFloatOrNull()
    }
}
