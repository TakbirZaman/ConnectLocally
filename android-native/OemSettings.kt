package com.example.localconnect
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
object OemSettings {
 fun open(activity:Activity) {
  val component=when(Build.MANUFACTURER.lowercase()) {
   "xiaomi" -> ComponentName("com.miui.securitycenter","com.miui.permcenter.autostart.AutoStartManagementActivity")
   "oppo" -> ComponentName("com.coloros.safecenter","com.coloros.safecenter.permission.startup.StartupAppListActivity")
   "vivo" -> ComponentName("com.vivo.permissionmanager","com.vivo.permissionmanager.activity.BgStartUpManagerActivity")
   else -> null
  }
  val candidates=listOfNotNull(component?.let {Intent().setComponent(it)},Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS),Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,Uri.parse("package:${activity.packageName}")))
  for(i in candidates) {try {if(i.resolveActivity(activity.packageManager)!=null){activity.startActivity(i);return}}catch(_:Exception){}}
 }
}
