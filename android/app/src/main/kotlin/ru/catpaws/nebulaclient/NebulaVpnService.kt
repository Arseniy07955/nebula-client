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
            val (net, len) = ConfigPrep.route(d.cidr)
            val b = Builder()
                .setSession(getString(R.string.app_name))
                .addAddress(d.ip, d.prefix)
                .addRoute(net, len)
                .setMtu(1300)
                .setBlocking(false)
            // DNS сети (например, 10.0.10.7) отвечает и на *.neb, и на обычные имена. Android не умеет слать в VPN-DNS
            // только отдельные зоны, поэтому на время подключения он обслуживает все запросы. Сервер вне сети не берём: туда нет маршрута.
            if (d.dnsOn) {
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
    }

    /** При смене сети (Wi-Fi <-> мобильная) сокет Nebula надо перевесить на новый интерфейс. */
    private fun watchNetwork() {
        if (netCb != null) return
        val cm = getSystemService(ConnectivityManager::class.java)
        var first = true
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                if (first) {
                    first = false
                    return
                }
                Nebulacore.rebindUDP()
            }
        }
        netCb = cb
        runCatching { cm.registerDefaultNetworkCallback(cb) }
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
