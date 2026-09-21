// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.feature.navigation

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import cl.baze.core.domain.HazardProximityUseCase
import cl.baze.core.location.LocationProvider
import cl.baze.core.network.BazeApiClient
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.koin.core.component.KoinComponent
import org.koin.core.component.inject

/**
 * Servicio en primer plano (foreground service) para navegación ciclista continua.
 * - Mantiene el rastreo de ubicación mediante LocationManager sin Google Play Services.
 * - Mantiene la conexión SSE de eventos en tiempo real sin requerir servicios de push privativos.
 */
class NavigationService : Service(), KoinComponent {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    // Inyección de dependencias mediante Koin
    private val locationProvider: LocationProvider by inject()
    private val apiClient: BazeApiClient by inject()
    private val proximityUseCase: HazardProximityUseCase by inject()

    companion object {
        const val CHANNEL_ID = "baze_navigation_channel"
        const val NOTIFICATION_ID = 1001
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        startForeground(NOTIFICATION_ID, buildForegroundNotification())

        // Iniciar observación de ubicación y alertas
        serviceScope.launch {
            locationProvider.observeLocation().collect { coords ->
                // TODO(verify): Evaluar proximidad con proximityUseCase y emitir notificación sonora
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Reducir riesgo de terminación por el sistema operativo durante la ruta activa
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        super.onDestroy()
        serviceScope.cancel()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Navegación Activa Baze",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Muestra el estado de la ruta y alertas de seguridad vial activas"
            }
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }

    private fun buildForegroundNotification(): Notification {
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Baze Navegación")
            .setContentText("Guía de ruta ciclista activa")
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setOngoing(true)
            .build()
    }
}
