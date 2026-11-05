package ru.catpaws.nebulaclient

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import ru.catpaws.nebula.nebulacore.Nebulacore
import java.util.concurrent.TimeUnit

/** Раз в несколько часов продлевает сертификат: без этого он кончится через срок, заданный в панели. */
class SyncWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        if (Store.load(applicationContext) == null) return Result.success()
        return try {
            val changed = Repo.sync(applicationContext)
            if (changed && Nebulacore.running()) NebulaVpnService.restart(applicationContext)
            Result.success()
        } catch (e: PanelError) {
            // 401/403: панель отключила устройство, повторять бессмысленно
            if (e.code == 401 || e.code == 403) Result.failure() else Result.retry()
        }
    }

    companion object {
        fun schedule(c: Context) {
            val req = PeriodicWorkRequestBuilder<SyncWorker>(6, TimeUnit.HOURS)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(c).enqueueUniquePeriodicWork("sync", ExistingPeriodicWorkPolicy.KEEP, req)
        }

        fun cancel(c: Context) {
            WorkManager.getInstance(c).cancelUniqueWork("sync")
        }
    }
}
