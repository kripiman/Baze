// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import cl.baze.app.R
import cl.baze.core.model.Coordinates
import cl.baze.feature.map.MapScreen
import cl.baze.feature.reports.ReportHazardDialog
import cl.baze.feature.search.SearchBar

@Composable
fun BazeApp() {
    var showReportDialog by remember { mutableStateOf(false) }

    Scaffold(
        modifier = Modifier.fillMaxSize(),
        floatingActionButton = {
            FloatingActionButton(
                onClick = { showReportDialog = true },
                containerColor = MaterialTheme.colorScheme.primaryContainer
            ) {
                Icon(
                    imageVector = Icons.Default.Warning,
                    contentDescription = "Reportar peligro vial"
                )
            }
        }
    ) { paddingValues ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
        ) {
            // Mapa base offline MapLibre
            MapScreen(modifier = Modifier.fillMaxSize())

            // Barra de búsqueda flotante superior
            SearchBar(
                modifier = Modifier
                    .align(Alignment.TopCenter)
                    .padding(horizontal = 16.dp, vertical = 8.dp)
            )

            // Atribución requerida por ODbL y OpenStreetMap
            Text(
                text = stringResource(R.string.attribution_notice),
                fontSize = 11.sp,
                color = Color.DarkGray,
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .background(Color.White.copy(alpha = 0.85f))
                    .padding(horizontal = 8.dp, vertical = 4.dp)
            )

            if (showReportDialog) {
                ReportHazardDialog(
                    location = Coordinates(longitude = -70.65, latitude = -33.45),
                    onDismiss = { showReportDialog = false },
                    onSubmit = { category, description ->
                        // TODO(verify): Conectar con ViewModel para invocar BazeApi.reportHazard
                        showReportDialog = false
                    }
                )
            }
        }
    }
}
