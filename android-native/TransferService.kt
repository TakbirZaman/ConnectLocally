package com.example.localconnect

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.*

class TransferService: Service() {
    private var generation=0L
    private var multicast:WifiManager.MulticastLock?=null
    private var wake:PowerManager.WakeLock?=null
    companion object {
        private var instance:TransferService?=null
        fun available()=instance!=null
        fun bind(epoch:Long){instance?.generation=epoch}
        fun setBusy(busy:Boolean) {instance?.apply {if(busy) {if(wake?.isHeld != true) wake?.acquire(10*60*1000L)} else if(wake?.isHeld == true)wake?.release()}}
    }
    override fun onCreate() {
        super.onCreate()
        try {
        val nm=getSystemService(NotificationManager::class.java)
        if(Build.VERSION.SDK_INT>=26)nm.createNotificationChannel(NotificationChannel("transfer","LocalConnect active operation",NotificationManager.IMPORTANCE_LOW))
        val open=PendingIntent.getActivity(this,0,Intent(this,MainActivity::class.java).apply {addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)},PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop=PendingIntent.getService(this,1,Intent(this,TransferService::class.java).setAction("stop"),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val builder=if(Build.VERSION.SDK_INT>=26)Notification.Builder(this,"transfer") else Notification.Builder(this)
        val n=builder.setContentTitle("LocalConnect LAN transfers").setContentText("Listening for paired devices. Tap Stop to disconnect.")
            .setSmallIcon(android.R.drawable.stat_sys_upload).setOngoing(true).setContentIntent(open)
            .addAction(Notification.Action.Builder(null,"Stop",stop).build()).build()
        if(Build.VERSION.SDK_INT>=29)startForeground(42,n,ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE) else startForeground(42,n)
        val wifi=applicationContext.getSystemService(WIFI_SERVICE) as WifiManager
        multicast=wifi.createMulticastLock("LocalConnect discovery").apply {setReferenceCounted(false);acquire()}
        wake=(getSystemService(POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"LocalConnect:transfer").apply {setReferenceCounted(false)}
        instance=this
        NativeHooks.transferReady?.invoke(mapOf("ok" to true))
        }catch(e:Exception){NativeHooks.transferReady?.invoke(mapOf("ok" to false,"error" to "Foreground operation unavailable; keep app visible and check permissions: ${e.message}"));stopSelf()}
    }
    override fun onStartCommand(intent:Intent?,flags:Int,startId:Int):Int {
        if(intent?.action=="stop") {requestStop();stopSelf()}
        // No boot restart or permanent service promise; resume explicitly in UI.
        return START_NOT_STICKY
    }
    private fun requestStop() {
        RemoteAccessibilityService.authorize(null)
        stopService(Intent(this,ProjectionService::class.java))
        RustLifecycle.stopAll(generation)
        MainActivity.stopCore(generation)
    }
    override fun onTimeout(startId:Int,fgsType:Int) {requestStop();stopSelf()}
    override fun onDestroy() {requestStop();if(wake?.isHeld==true)wake?.release();if(multicast?.isHeld==true)multicast?.release();instance=null;RustLifecycle.stopAll(generation){NativeHooks.transferStopped?.invoke()};stopForeground(STOP_FOREGROUND_REMOVE);super.onDestroy()}
    override fun onBind(intent:Intent?):IBinder?=null
}
