package ru.catpaws.nebulaclient

import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import ru.catpaws.nebula.nebulacore.Nebulacore

/** Плитка в шторке: включить или выключить подключение, не открывая приложение. */
class NebulaTileService : TileService() {
    companion object {
        /** Попросить систему перечитать плитку (после подключения, отключения или ошибки). */
        fun refresh(c: Context) {
            runCatching { requestListeningState(c, ComponentName(c, NebulaTileService::class.java)) }
        }
    }

    override fun onStartListening() {
        super.onStartListening()
        update()
    }

    private fun update() {
        val tile = qsTile ?: return
        val d = Store.load(this)
        val status = VpnState.status.value
        tile.state = when {
            d == null -> Tile.STATE_UNAVAILABLE
            Nebulacore.running() || status is VpnStatus.On || status is VpnStatus.Connecting -> Tile.STATE_ACTIVE
            else -> Tile.STATE_INACTIVE
        }
        tile.label = getString(R.string.tile_label)
        if (Build.VERSION.SDK_INT >= 29) {
            tile.subtitle = when {
                d == null -> getString(R.string.tile_no_network)
                status is VpnStatus.Connecting -> getString(R.string.tile_connecting)
                tile.state == Tile.STATE_ACTIVE -> d.ip
                else -> d.title
            }
        }
        tile.updateTile()
    }

    override fun onClick() {
        super.onClick()
        val d = Store.load(this)
        val tile = qsTile
        if (d == null) return openApp(false)
        if (tile?.state == Tile.STATE_ACTIVE) {
            NebulaVpnService.disconnect(this)
            show(tile, Tile.STATE_INACTIVE)
            return
        }
        // разрешение VPN ещё не дано (или отозвано): его диалог открывается только из приложения, там же и подключимся
        if (VpnService.prepare(this) != null) return openApp(true)
        runCatching { NebulaVpnService.connect(this) }
            .onSuccess { show(tile, Tile.STATE_ACTIVE) }
            .onFailure { openApp(true) }
    }

    /** До ответа службы показываем ожидаемое состояние, потом его поправит refresh(). */
    private fun show(tile: Tile?, state: Int) {
        tile ?: return
        tile.state = state
        tile.updateTile()
    }

    private fun openApp(connect: Boolean) {
        if (connect) VpnState.askConnect(true)
        val i = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, i, PendingIntent.FLAG_IMMUTABLE))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(i)
        }
    }
}
