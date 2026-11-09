package ru.catpaws.nebulaclient

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import ru.catpaws.nebula.nebulacore.Nebulacore
import java.text.DateFormat
import java.util.Date

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        VpnState.tileCtx = applicationContext
        setContent {
            val dark = isSystemInDarkTheme()
            MaterialTheme(colorScheme = if (dark) darkColorScheme() else lightColorScheme()) {
                Surface(Modifier.fillMaxSize()) { App() }
            }
        }
    }
}

@Composable
fun App() {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var device by remember { mutableStateOf(Store.load(ctx)) }
    LaunchedEffect(Unit) {
        // KEEP оставляет уже стоящую работу, так что повторные открытия окна ничего не сбрасывают
        UpdateWorker.schedule(ctx)
        if (device != null) SyncWorker.schedule(ctx)
    }
    Box(Modifier.fillMaxSize().statusBarsPadding()) {
        val d = device
        if (d == null) {
            EnrollScreen { device = it; SyncWorker.schedule(ctx) }
        } else {
            MainScreen(d, onChanged = { device = Store.load(ctx) }, onLeft = { device = null; SyncWorker.cancel(ctx) })
        }
    }
}

private fun defaultName(): String {
    val raw = (Build.MODEL ?: "android").replace(Regex("[^A-Za-z0-9._-]"), "-").trim('-', '.', '_')
    return raw.ifEmpty { "android" }.take(63)
}

@Composable
fun EnrollScreen(onDone: (Device) -> Unit) {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    var server by remember { mutableStateOf(DEFAULT_SERVER) }
    var name by remember { mutableStateOf(defaultName()) }
    var invite by remember { mutableStateOf("") }
    var net by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Nebula Client", style = MaterialTheme.typography.headlineMedium)
        Text(
            tr(
                "Введите имя устройства и код приглашения от администратора сети.",
                "Enter a device name and the invite code from the network administrator.",
            ),
        )
        OutlinedTextField(
            name, { name = it }, label = { Text(tr("Имя устройства", "Device name")) },
            singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy,
        )
        OutlinedTextField(
            invite, { invite = it }, label = { Text(tr("Код приглашения", "Invite code")) },
            singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy,
        )
        OutlinedTextField(
            server, { server = it }, label = { Text(tr("Адрес панели", "Panel address")) },
            singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
        )
        OutlinedTextField(
            net, { net = it }, label = { Text(tr("Код сети (если их несколько)", "Network code (if there are several)")) },
            singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy,
        )
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Button(
            onClick = {
                busy = true
                error = null
                scope.launch {
                    val r = withContext(Dispatchers.IO) { runCatching { Repo.enroll(ctx, server, name, invite, net) } }
                    busy = false
                    r.onSuccess { onDone(it) }.onFailure { error = it.message ?: it.toString() }
                }
            },
            enabled = !busy && name.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text(tr("Войти в сеть", "Join the network"))
        }
    }
}

@Composable
fun MainScreen(d: Device, onChanged: () -> Unit, onLeft: () -> Unit) {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    val status by VpnState.status.collectAsStateWithLifecycle()
    val notice by VpnState.notice.collectAsStateWithLifecycle()
    var peers by remember { mutableStateOf(d.peers) }
    var tunnels by remember { mutableStateOf(0L) }
    var rtt by remember { mutableStateOf<Map<String, Float?>>(emptyMap()) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var message by remember { mutableStateOf<String?>(null) }
    var showLogs by remember { mutableStateOf(false) }
    var confirmLeave by remember { mutableStateOf(false) }
    var syncing by remember { mutableStateOf(false) }
    var showUpdates by remember { mutableStateOf(false) }
    val upd by Updater.state.collectAsStateWithLifecycle()

    val notifPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
    val vpnPerm = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        if (r.resultCode == android.app.Activity.RESULT_OK) NebulaVpnService.connect(ctx)
        else message = tr("Без разрешения VPN подключиться нельзя", "Cannot connect without the VPN permission")
    }

    fun connect() {
        message = null
        if (Build.VERSION.SDK_INT >= 33 &&
            ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            notifPerm.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        val intent: Intent? = VpnService.prepare(ctx)
        if (intent != null) vpnPerm.launch(intent) else NebulaVpnService.connect(ctx)
    }

    // плитка в шторке попросила подключиться, а разрешения VPN не было
    val asked by VpnState.connectAsked.collectAsStateWithLifecycle()
    LaunchedEffect(asked) {
        if (asked) {
            VpnState.askConnect(false)
            if (status !is VpnStatus.On && status !is VpnStatus.Connecting) connect()
        }
    }

    // служба могла жить без окна (после «Постоянной сети VPN»), поэтому сверяем статус с ядром
    LaunchedEffect(Unit) {
        if (status is VpnStatus.Off && Nebulacore.running()) {
            VpnState.set(VpnStatus.On)
        } else if (status is VpnStatus.Off && d.wantUp && VpnService.prepare(ctx) == null) {
            // подключение было включено, но процесс умер (например, приложение обновилось): поднимаем снова
            NebulaVpnService.connect(ctx)
        }
    }
    // только пока окно на экране: в фоне ни панель, ни участников не дёргаем
    LaunchedEffect(status) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                if (status is VpnStatus.On) tunnels = Nebulacore.tunnels()
                peers = withContext(Dispatchers.IO) { Repo.peers(ctx) }
                delay(15_000)
            }
        }
    }
    LaunchedEffect(status, peers) {
        if (status !is VpnStatus.On) {
            rtt = emptyMap()
            return@LaunchedEffect
        }
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                rtt = Ping.all(peers.filter { it.online && it.ip != d.ip }.map { it.ip })
                delay(5_000)
            }
        }
    }

    val on = status is VpnStatus.On
    val connecting = status is VpnStatus.Connecting
    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Spacer(Modifier.height(8.dp))
        Text(d.title, style = MaterialTheme.typography.headlineMedium)
        Text(
            "${d.name} · ${d.ip}", fontFamily = FontFamily.Monospace,
            modifier = Modifier.clickable { copyText(ctx, d.ip) },
        )

        Card(Modifier.fillMaxWidth()) {
            Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        when (status) {
                            is VpnStatus.On -> tr("Подключено", "Connected")
                            is VpnStatus.Connecting -> tr("Подключаюсь…", "Connecting…")
                            is VpnStatus.Error -> tr("Ошибка", "Error")
                            else -> tr("Отключено", "Disconnected")
                        },
                        style = MaterialTheme.typography.titleMedium,
                    )
                    if (on) Text(tr("Туннелей: $tunnels", "Tunnels: $tunnels"), style = MaterialTheme.typography.bodySmall)
                }
                Switch(
                    checked = on || connecting,
                    onCheckedChange = { want -> if (want) connect() else NebulaVpnService.disconnect(ctx) },
                )
            }
        }
        (status as? VpnStatus.Error)?.let { Text(it.message, color = MaterialTheme.colorScheme.error) }
        message?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        notice?.let { Text(it, color = MaterialTheme.colorScheme.tertiary, style = MaterialTheme.typography.bodySmall) }

        val fmt = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
        Text(
            tr("Сертификат действует до ", "Certificate valid until ") + fmt.format(Date(d.notAfter * 1000)),
            style = MaterialTheme.typography.bodySmall,
        )

        if (d.dns.isNotEmpty()) {
            var dnsOn by remember(d.dnsOn) { mutableStateOf(d.dnsOn) }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(tr("DNS сети", "Network DNS"))
                    Text(
                        d.dns.joinToString(", ") + if (d.dnsDomains.isNotEmpty()) " (*." + d.dnsDomains.joinToString(", *.") + ")" else "",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                Switch(checked = dnsOn, onCheckedChange = {
                    dnsOn = it
                    Repo.setDnsOn(ctx, it)
                    onChanged()
                    if (Nebulacore.running()) NebulaVpnService.restart(ctx)
                })
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = {
                    syncing = true
                    scope.launch {
                        val r = withContext(Dispatchers.IO) { runCatching { Repo.sync(ctx) } }
                        syncing = false
                        r.onSuccess { changed ->
                            message = null
                            onChanged()
                            if (changed && Nebulacore.running()) NebulaVpnService.restart(ctx)
                        }.onFailure { message = it.message }
                    }
                },
                enabled = !syncing,
            ) { Text(tr("Обновить", "Refresh")) }
            OutlinedButton(onClick = { showLogs = true }) { Text(tr("Журнал", "Log")) }
            OutlinedButton(onClick = { confirmLeave = true }) { Text(tr("Выйти", "Leave")) }
        }
        val avail = (upd as? UpdState.Available)?.release
        OutlinedButton(onClick = { showUpdates = true }, modifier = Modifier.fillMaxWidth()) {
            Text(if (avail != null) tr("Обновления: доступна ${avail.version}", "Updates: ${avail.version} available") else tr("Обновления", "Updates"))
        }

        val online = peers.count { it.online }
        Row(verticalAlignment = Alignment.Bottom) {
            Text(tr("Участники ", "Members ") + "$online/${peers.size}", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            Text(tr("нажмите, чтобы скопировать IP", "tap to copy the IP"), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline)
        }
        // вы, потом кто в сети, потом по имени; по пингу не сортируем, иначе строки прыгают
        val sorted = remember(peers, d.ip) { peers.sortedWith(compareBy({ it.ip != d.ip }, { !it.online }, { it.name.lowercase() })) }
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            items(sorted, key = { it.ip }) { p ->
                Row(
                    Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small).clickable { copyText(ctx, p.ip) }.padding(vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier.size(10.dp).clip(CircleShape)
                            .background(if (p.online) Color(0xFF2EAD4B) else Color(0xFF8A8A8A)),
                    )
                    Spacer(Modifier.size(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text(p.name)
                        Text(p.ip, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
                    }
                    when {
                        p.ip == d.ip -> Text(tr("это вы", "you"), style = MaterialTheme.typography.bodySmall)
                        p.role == "lighthouse" -> Text(tr("маяк", "lighthouse"), style = MaterialTheme.typography.bodySmall)
                        p.role.isNotEmpty() && p.role != "client" -> Text(p.role, style = MaterialTheme.typography.bodySmall)
                    }
                    rtt[p.ip]?.let { ms ->
                        Spacer(Modifier.size(10.dp))
                        Text(
                            if (ms < 1f) tr("<1 мс", "<1 ms") else tr("${ms.toInt()} мс", "${ms.toInt()} ms"),
                            style = MaterialTheme.typography.bodySmall,
                            color = when {
                                ms < 60f -> Color(0xFF2EAD4B)
                                ms < 150f -> Color(0xFFE5A50A)
                                else -> Color(0xFFE04F4F)
                            },
                        )
                    }
                }
            }
        }
    }

    if (showUpdates) UpdatesDialog(onClose = { showUpdates = false })
    if (showLogs) {
        AlertDialog(
            onDismissRequest = { showLogs = false },
            confirmButton = { TextButton(onClick = { showLogs = false }) { Text(tr("Закрыть", "Close")) } },
            title = { Text(tr("Журнал Nebula", "Nebula log")) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    Text(Nebulacore.logs().takeLast(8000).ifEmpty { "-" }, fontFamily = FontFamily.Monospace, fontSize = 10.sp)
                }
            },
        )
    }
    if (confirmLeave) {
        AlertDialog(
            onDismissRequest = { confirmLeave = false },
            title = { Text(tr("Выйти из сети?", "Leave the network?")) },
            text = {
                Text(
                    tr(
                        "Ключ устройства сотрётся, сертификат будет отозван на панели. Чтобы вернуться, нужен новый инвайт.",
                        "The device key will be deleted and the certificate revoked on the panel. You will need a new invite to return.",
                    ),
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmLeave = false
                    NebulaVpnService.disconnect(ctx)
                    scope.launch {
                        withContext(Dispatchers.IO) { Repo.leave(ctx) }
                        onLeft()
                    }
                }) { Text(tr("Выйти", "Leave")) }
            },
            dismissButton = { TextButton(onClick = { confirmLeave = false }) { Text(tr("Отмена", "Cancel")) } },
        )
    }
}

/** Скопировать адрес. С Android 13 система сама показывает, что скопировано. */
fun copyText(ctx: android.content.Context, text: String) {
    val cm = ctx.getSystemService(android.content.ClipboardManager::class.java)
    cm.setPrimaryClip(android.content.ClipData.newPlainText("IP", text))
    if (Build.VERSION.SDK_INT < 33) {
        android.widget.Toast.makeText(ctx, tr("Скопировано: ", "Copied: ") + text, android.widget.Toast.LENGTH_SHORT).show()
    }
}

@Composable
fun UpdatesDialog(onClose: () -> Unit) {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    val upd by Updater.state.collectAsStateWithLifecycle()
    var mode by remember { mutableStateOf(Updater.mode(ctx)) }
    var pre by remember { mutableStateOf(Updater.prerelease(ctx)) }
    var canInstall by remember { mutableStateOf(Updater.canInstall(ctx)) }
    val fmt = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)

    // после возврата из окна разрешения «Неизвестные приложения» сверяем его заново
    androidx.compose.runtime.DisposableEffect(Unit) {
        val obs = androidx.lifecycle.LifecycleEventObserver { _, e ->
            if (e == androidx.lifecycle.Lifecycle.Event.ON_RESUME) canInstall = Updater.canInstall(ctx)
        }
        val lc = (ctx as androidx.lifecycle.LifecycleOwner).lifecycle
        lc.addObserver(obs)
        onDispose { lc.removeObserver(obs) }
    }

    fun check() {
        Updater.set(UpdState.Checking)
        scope.launch {
            val r = withContext(Dispatchers.IO) { runCatching { Updater.check(ctx) } }
            r.onSuccess { Updater.set(if (it != null) UpdState.Available(it) else UpdState.UpToDate) }
                .onFailure { Updater.set(UpdState.Error(it.message ?: it.toString())) }
        }
    }

    fun install(rel: Release) {
        if (!Updater.canInstall(ctx)) {
            ctx.startActivity(Updater.unknownSourcesIntent(ctx))
            return
        }
        scope.launch { withContext(Dispatchers.IO) { Updater.downloadAndInstall(ctx, rel) } }
    }

    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = { TextButton(onClick = onClose) { Text(tr("Закрыть", "Close")) } },
        title = { Text(tr("Обновления", "Updates")) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(tr("Установлена версия ", "Installed version ") + Updater.installedVersion(ctx))
                val last = Updater.lastCheck(ctx)
                if (last > 0) Text(tr("Проверено: ", "Last check: ") + fmt.format(Date(last * 1000)), style = MaterialTheme.typography.bodySmall)
                when (val u = upd) {
                    is UpdState.Checking -> Text(tr("Проверяю…", "Checking…"))
                    is UpdState.UpToDate -> Text(tr("У вас последняя версия", "You are up to date"))
                    is UpdState.Available -> {
                        Text(tr("Доступна версия ", "Version available: ") + u.release.version)
                        Button(onClick = { install(u.release) }) { Text(tr("Обновить", "Update")) }
                    }
                    is UpdState.Downloading -> Text(tr("Скачиваю ${u.release.version}: ${u.percent}%", "Downloading ${u.release.version}: ${u.percent}%"))
                    is UpdState.Installing -> Text(tr("Устанавливаю ${u.release.version}…", "Installing ${u.release.version}…"))
                    is UpdState.Error -> Text(u.message, color = MaterialTheme.colorScheme.error)
                    else -> {}
                }
                if (upd !is UpdState.Checking) OutlinedButton(onClick = { check() }) { Text(tr("Проверить сейчас", "Check now")) }

                Text(tr("Как обновлять", "Update mode"), style = MaterialTheme.typography.titleSmall)
                listOf(
                    UpdateMode.AUTO to tr("Само", "Automatically"),
                    UpdateMode.NOTIFY to tr("Только сообщать", "Notify only"),
                    UpdateMode.OFF to tr("Не проверять", "Never check"),
                ).forEach { (m, label) ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        androidx.compose.material3.RadioButton(selected = mode == m, onClick = { mode = m; Updater.setMode(ctx, m) })
                        Text(label)
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    androidx.compose.material3.Checkbox(checked = pre, onCheckedChange = { pre = it; Updater.setPrerelease(ctx, it) })
                    Text(tr("Тестовые версии (pre-release)", "Test versions (pre-release)"))
                }
                if (!canInstall) {
                    Text(
                        tr(
                            "Чтобы приложение ставило обновления, разрешите ему установку неизвестных приложений.",
                            "To install updates the app needs the \"install unknown apps\" permission.",
                        ),
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.tertiary,
                    )
                    OutlinedButton(onClick = { ctx.startActivity(Updater.unknownSourcesIntent(ctx)) }) { Text(tr("Открыть настройки", "Open settings")) }
                }
            }
        },
    )
}
