package ru.catpaws.nebulaclient

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.VpnService
import android.os.Build
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import ru.catpaws.nebula.nebulacore.Nebulacore
import java.util.concurrent.Executors

sealed class VpnStatus {
    data object Off : VpnStatus()
    data object Connecting : VpnStatus()
    data object On : VpnStatus()
    data class Error(val message: String) : VpnStatus()
}

object VpnState {
    private val _status = MutableStateFlow<VpnStatus>(VpnStatus.Off)
    val status: StateFlow<VpnStatus> = _status
    fun set(s: VpnStatus) {
        _status.value = s
    }

    /** Предупреждение под переключателем (например, DNS сети не включён из-за «Частного DNS»). */
    private val _notice = MutableStateFlow<String?>(null)
    val notice: StateFlow<String?> = _notice
    fun setNotice(s: String?) {
        _notice.value = s
    }
}

/**
 * Nebula работает внутри этого процесса: VpnService выдаёт адаптер (tun), его дескриптор уходит в Go-ядро.
 * Системная иконка ключа в статус-баре и запрос разрешения VPN приходят от Android.
 */
class NebulaVpnService : VpnService() {
    companion object {
        const val ACTION_CONNECT = "ru.catpaws.nebulaclient.CONNECT"
        const val ACTION_DISCONNECT = "ru.catpaws.nebulaclient.DISCONNECT"
        const val ACTION_RESTART = "ru.catpaws.nebulaclient.RESTART"
        private const val CHANNEL = "vpn"
        private const val NOTIF_ID = 1

        fun connect(c: Context) = start(c, ACTION_CONNECT)
        fun disconnect(c: Context) = start(c, ACTION_DISCONNECT)
        fun restart(c: Context) = start(c, ACTION_RESTART)

        private fun start(c: Context, action: String) {
            val i = Intent(c, NebulaVpnService::class.java).setAction(action)
            // служба VPN после запроса разрешения может стартовать из фона; для остановки foreground не нужен
            if (action == ACTION_DISCONNECT) c.startService(i) else c.startForegroundService(i)
        }
    }

    private val worker = Executors.newSingleThreadExecutor()
    private var netCb: ConnectivityManager.NetworkCallback? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_DISCONNECT -> {
                Repo.setWant(this, false)
                worker.execute { stopTunnel(); stopSelf() }
            }
            ACTION_RESTART -> worker.execute { stopTunnel(); startTunnel() }
            // ACTION_CONNECT или null: так Android запускает службу для «Постоянной сети VPN»
            else -> worker.execute { startTunnel() }
        }
        return START_STICKY
    }

    private fun notification(text: String): Notification {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.notif_channel), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val off = PendingIntent.getService(
            this, 1, Intent(this, NebulaVpnService::class.java).setAction(ACTION_DISCONNECT), PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setContentIntent(open)
            .addAction(0, getString(R.string.notif_disconnect), off)
            .setOngoing(true)
            .build()
    }

    private fun foreground(text: String) {
        val n = notification(text)
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIF_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED)
        } else {
            startForeground(NOTIF_ID, n)
        }
    }

    @Synchronized
    private fun startTunnel() {
        foreground(tr("Подключаюсь…", "Connecting…"))
        val d0 = Store.load(this)
        if (d0 == null) {
            VpnState.set(VpnStatus.Error(tr("Устройство ещё не в сети", "This device is not in a network yet")))
            stopSelf()
            return
        }
        VpnState.set(VpnStatus.Connecting)
        Repo.setWant(this, true)
        try {
            // сертификат на исходе: сначала продлеваем, пока интернет идёт мимо туннеля
            if (d0.notAfter - System.currentTimeMillis() / 1000 < 24 * 3600) {
                runCatching { Repo.sync(this) }
            }
            val d = Store.load(this) ?: d0
            if (Nebulacore.running()) Nebulacore.stop()
            refreshDns()
            val (net, len) = ConfigPrep.route(d.cidr)
            val b = Builder()
                .setSession(getString(R.string.app_name))
                .addAddress(d.ip, d.prefix)
                .addRoute(net, len)
                .setMtu(1300)
                .setBlocking(false)
            // DNS сети (например, 10.0.10.7) отвечает и на *.neb, и на обычные имена. Android не умеет слать в VPN-DNS
            // только отдельные зоны, поэтому на время подключения он обслуживает все запросы. Сервер вне сети не берём: туда нет маршрута.
            // Строгий «Частный DNS» (имя хоста): Android шлёт тогда все запросы по DoT на этот хост через VPN-сеть, а маршрута
            // до него нет, и перестают резолвиться вообще все имена. Поэтому в этом режиме DNS сети в VPN не ставим.
            // Если этот хост и есть DNS сети (тот же сервер), он сам отвечает на *.neb, и ничего больше не нужно.
            val strict = if (d.dnsOn && d.dns.isNotEmpty()) strictPrivateDns() else null
            val same = strict != null && privateDnsIsNetworkDns(strict, d.dns)
            VpnState.setNotice(
                when {
                    strict == null -> null
                    same -> tr(
                        "DNS сети обслуживает ваш «Частный DNS» $strict (тот же сервер), отдельный DNS в VPN не нужен.",
                        "Your Private DNS $strict is the network DNS server itself, so no separate VPN DNS is needed.",
                    )
                    else -> tr(
                        "DNS сети не включён: в Android стоит «Частный DNS» $strict, с ним имена .neb не резолвятся. " +
                            "Выберите «Автоматически» или «Отключено» в настройках сети и переподключитесь.",
                        "Network DNS is off: Android Private DNS is set to $strict, which breaks .neb names. " +
                            "Set Private DNS to Automatic or Off in the network settings and reconnect.",
                    )
                },
            )
            if (d.dnsOn && strict == null) {
                d.dns.filter { inCidr(it, net, len) }.forEach { b.addDnsServer(it) }
            }
            val pfd = b.establish()
                ?: throw IllegalStateException(tr("Android не дал создать VPN: разрешение отозвано", "Android refused to create the VPN: permission was revoked"))
            // дескриптор переходит к Go: он закроет его при остановке
            Nebulacore.start(ConfigPrep.prepare(d.panelConfig, d), pfd.detachFd().toLong())
            watchNetwork()
            VpnState.set(VpnStatus.On)
            val nm = getSystemService(NotificationManager::class.java)
            nm.notify(NOTIF_ID, notification(getString(R.string.notif_text, d.title, d.ip)))
        } catch (e: Throwable) {
            VpnState.set(VpnStatus.Error(e.message ?: e.javaClass.simpleName))
            Repo.setWant(this, false)
            runCatching { Nebulacore.stop() }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    /** Имя хоста строгого «Частного DNS» на обычной (не VPN) сети или null. */
    private fun strictPrivateDns(): String? {
        if (Build.VERSION.SDK_INT < 28) return null
        val cm = getSystemService(ConnectivityManager::class.java)
        for (n in cm.allNetworks) {
            val c = cm.getNetworkCapabilities(n) ?: continue
            if (c.hasTransport(NetworkCapabilities.TRANSPORT_VPN) || !c.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) continue
            val lp = cm.getLinkProperties(n) ?: continue
            val name = lp.privateDnsServerName
            if (lp.isPrivateDnsActive && !name.isNullOrEmpty()) return name
        }
        return null
    }

    /** Резолвится ли хост «Частного DNS» в адрес одного из DNS-серверов сети (до подъёма VPN, по обычной сети). */
    private fun privateDnsIsNetworkDns(host: String, netDns: List<String>): Boolean =
        runCatching { java.net.InetAddress.getAllByName(host).any { it.hostAddress in netDns } }.getOrDefault(false)

    private fun inCidr(ip: String, net: String, len: Int): Boolean = runCatching {
        fun v(a: String) = a.split(".").fold(0L) { acc, x -> (acc shl 8) or x.toLong() }
        val mask = if (len == 0) 0L else (0xFFFFFFFFL shl (32 - len)) and 0xFFFFFFFFL
        (v(ip) and mask) == (v(net) and mask)
    }.getOrDefault(false)

    @Synchronized
    private fun stopTunnel() {
        unwatchNetwork()
        runCatching { Nebulacore.stop() }
        VpnState.set(VpnStatus.Off)
        VpnState.setNotice(null)
    }

    /** DNS обычных сетей (не VPN): ядру нужны они, чтобы разрешать имена маяков. Пока VPN поднят, активная сеть уже наша, поэтому ищем среди всех. */
    private fun refreshDns() {
        val cm = getSystemService(ConnectivityManager::class.java)
        val servers = cm.allNetworks.filter {
            val c = cm.getNetworkCapabilities(it)
            c != null && !c.hasTransport(NetworkCapabilities.TRANSPORT_VPN) && c.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        }.flatMap { cm.getLinkProperties(it)?.dnsServers.orEmpty() }.mapNotNull { it.hostAddress }.distinct()
        Nebulacore.setDNS(servers.joinToString(","))
    }

    /** При смене сети (Wi-Fi <-> мобильная) сокет Nebula надо перевесить на новый интерфейс. */
    private fun watchNetwork() {
        if (netCb != null) return
        val cm = getSystemService(ConnectivityManager::class.java)
        var first = true
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                refreshDns()
                if (first) {
                    first = false
                    return
                }
                Nebulacore.rebindUDP()
            }
        }
        netCb = cb
        val req = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        runCatching { cm.registerNetworkCallback(req, cb) }
    }

    private fun unwatchNetwork() {
        val cm = getSystemService(ConnectivityManager::class.java)
        netCb?.let { runCatching { cm.unregisterNetworkCallback(it) } }
        netCb = null
    }

    override fun onRevoke() {
        // пользователь включил другой VPN или выключил наш в системных настройках
        Repo.setWant(this, false)
        worker.execute { stopTunnel(); stopSelf() }
    }

    override fun onDestroy() {
        stopTunnel()
        worker.shutdown()
        super.onDestroy()
    }
}
