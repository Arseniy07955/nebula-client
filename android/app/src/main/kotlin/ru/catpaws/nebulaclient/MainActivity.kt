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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
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
    var peers by remember { mutableStateOf(d.peers) }
    var tunnels by remember { mutableStateOf(0L) }
    var message by remember { mutableStateOf<String?>(null) }
    var showLogs by remember { mutableStateOf(false) }
    var confirmLeave by remember { mutableStateOf(false) }
    var syncing by remember { mutableStateOf(false) }

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

    // служба могла жить без окна (после «Постоянной сети VPN»), поэтому сверяем статус с ядром
    LaunchedEffect(Unit) {
        if (status is VpnStatus.Off && Nebulacore.running()) VpnState.set(VpnStatus.On)
    }
    LaunchedEffect(status) {
        while (true) {
            if (status is VpnStatus.On) tunnels = Nebulacore.tunnels()
            peers = withContext(Dispatchers.IO) { Repo.peers(ctx) }
            delay(15_000)
        }
    }

    val on = status is VpnStatus.On
    val connecting = status is VpnStatus.Connecting
    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Spacer(Modifier.height(8.dp))
        Text(d.title, style = MaterialTheme.typography.headlineMedium)
        Text("${d.name} · ${d.ip}", fontFamily = FontFamily.Monospace)

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

        Text(tr("Участники", "Members"), style = MaterialTheme.typography.titleMedium)
        LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            items(peers) { p ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(
                        Modifier.size(10.dp).clip(CircleShape)
                            .background(if (p.online) Color(0xFF2EAD4B) else Color(0xFF8A8A8A)),
                    )
                    Spacer(Modifier.size(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text(p.name)
                        Text(p.ip, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
                    }
                    if (p.role.isNotEmpty() && p.role != "client") Text(p.role, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }

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
