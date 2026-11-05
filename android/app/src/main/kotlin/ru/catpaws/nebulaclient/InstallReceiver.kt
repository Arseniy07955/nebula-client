package ru.catpaws.nebulaclient

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller

/** Результат установки от PackageInstaller. Успех приложение само не видит: его процесс заменяется новым. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
        val status = i.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                // Android просит подтверждение (первое обновление или не тот установщик)
                val confirm = if (android.os.Build.VERSION.SDK_INT >= 33) {
                    i.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    @Suppress("DEPRECATION") i.getParcelableExtra(Intent.EXTRA_INTENT)
                } ?: return
                confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                runCatching { c.startActivity(confirm) }
                // если окно не открылось из фона, подтверждение остаётся в уведомлении
                Updater.notify(
                    c, 2, tr("Обновление Nebula Client", "Nebula Client update"),
                    tr("Нажмите, чтобы подтвердить установку", "Tap to confirm the installation"),
                    PendingIntent.getActivity(c, 1, confirm, PendingIntent.FLAG_IMMUTABLE),
                )
            }
            PackageInstaller.STATUS_SUCCESS -> Updater.set(UpdState.Idle)
            else -> {
                val msg = i.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "status $status"
                Updater.set(UpdState.Error(msg))
            }
        }
    }
}
