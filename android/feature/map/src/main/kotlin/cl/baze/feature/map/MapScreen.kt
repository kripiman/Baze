// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.feature.map

import android.view.ViewGroup
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import cl.baze.core.model.Coordinates
import org.maplibre.android.MapLibre
import org.maplibre.android.camera.CameraPosition
import org.maplibre.android.geometry.LatLng
import org.maplibre.android.maps.MapView
import java.io.File

/**
 * Contenedor Composable para MapLibre Native Android.
 * Carga el mapa vectorial base offline mediante el protocolo local pmtiles://file://.
 */
@Composable
fun MapScreen(
    modifier: Modifier = Modifier,
    initialCenter: Coordinates = Coordinates(longitude = -70.65, latitude = -33.45), // Santiago por defecto
    onMapReady: () -> Unit = {}
) {
    val context = LocalContext.current
    remember { MapLibre.getInstance(context) }

    val mapView = remember {
        MapView(context).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        }
    }

    DisposableEffect(mapView) {
        mapView.onCreate(null)
        mapView.onStart()
        mapView.onResume()

        onDispose {
            mapView.onPause()
            mapView.onStop()
            mapView.onDestroy()
        }
    }

    AndroidView(
        modifier = modifier.fillMaxSize(),
        factory = {
            mapView.apply {
                getMapAsync { mapboxMap ->
                    // Ruta local al archivo PMTiles descargado
                    val pmtilesFile = File(context.filesDir, "tiles.pmtiles")
                    // TODO(verify): Verificar integración del protocolo de teselas pmtiles:// en MapLibre Android
                    val styleUrl = if (pmtilesFile.exists()) {
                        "pmtiles://file://${pmtilesFile.absolutePath}"
                    } else {
                        "asset://styles/style.json"
                    }

                    mapboxMap.setStyle(styleUrl) {
                        mapboxMap.cameraPosition = CameraPosition.Builder()
                            .target(LatLng(initialCenter.latitude, initialCenter.longitude))
                            .zoom(14.0)
                            .build()
                        onMapReady()
                    }
                }
            }
        }
    )
}
