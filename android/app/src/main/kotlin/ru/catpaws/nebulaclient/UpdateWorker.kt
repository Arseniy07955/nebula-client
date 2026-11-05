package ru.catpaws.nebulaclient

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/** Раз в 12 часов смотрит релизы: в режиме «само» ставит, в режиме «сообщать» присылает уведомление. */
class UpdateWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        val c = applicationContext
        val mode = Updater.mode(c)
        if (mode == UpdateMode.OFF) return Result.success()
        val r = try {
            Updater.check(c)
        } catch (e: Exception) {
            return Result.retry()
        } ?: return Result.success()

        if (mode == UpdateMode.AUTO && Updater.canInstall(c)) {
            Updater.downloadAndInstall(c, r)
        } else {
            Updater.set(UpdState.Available(r))
            Updater.notify(
                c, 3, tr("Доступна версия ${r.version}", "Version ${r.version} is available"),
                tr("Откройте приложение, чтобы обновить", "Open the app to update"), Updater.openAppIntent(c),
            )
        }
        return Result.success()
    }

    companion object {
        fun schedule(c: Context) {
            val req = PeriodicWorkRequestBuilder<UpdateWorker>(12, TimeUnit.HOURS)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(c).enqueueUniquePeriodicWork("update", ExistingPeriodicWorkPolicy.KEEP, req)
        }
    }
}
