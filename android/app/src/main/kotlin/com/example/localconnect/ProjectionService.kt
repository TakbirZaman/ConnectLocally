package com.example.localconnect

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.graphics.*
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.*
import java.io.File
import java.nio.ByteBuffer
import kotlin.math.min

/** One fresh OS consent token per instance; resize the same virtual display. */
class ProjectionService:Service() {
    companion object {@Volatile var ticket:String?=null;private set}
    private var projection:MediaProjection?=null
    private var virtual:VirtualDisplay?=null
    private var reader:ImageReader?=null
    private var thread:HandlerThread?=null
    private var handler:Handler?=null
    private var wake:PowerManager.WakeLock?=null
    private var used=false
    @Volatile private var stopped=false
    private var last=0L
    private var sequence=0L
    private var display=emptyMap<String,Any>()
    private val main=Handler(Looper.getMainLooper())
    private val projectionCallback=object:MediaProjection.Callback(){
        override fun onStop(){stopSelf()}
        override fun onCapturedContentResize(width:Int,height:Int){if(!stopped)main.post{resize()}}
    }
    private val renew=object:Runnable{override fun run(){if(!stopped&&projection!=null){wake?.acquire(10*60*1000L);main.postDelayed(this,60_000)}}}
    override fun onCreate(){
        super.onCreate()
        val manager=getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("projection","LocalConnect screen sharing",NotificationManager.IMPORTANCE_LOW))
        val stop=PendingIntent.getService(this,44,Intent(this,ProjectionService::class.java).setAction("stop"),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val notification=Notification.Builder(this,"projection").setContentTitle("LocalConnect screen sharing active").setContentText("User-approved LAN session. Stop revokes screen sharing and input.").setSmallIcon(android.R.drawable.stat_sys_upload).setOngoing(true).addAction(Notification.Action.Builder(null,"Stop sharing",stop).build()).build()
        try{if(Build.VERSION.SDK_INT>=29)startForeground(43,notification,ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)else startForeground(43,notification)}
        catch(e:Exception){stopped=true;NativeHooks.projectionReady?.invoke(mapOf("ok" to false,"error" to "Keep the app visible and grant screen capture consent: ${e.message}"));stopSelf()}
    }
    override fun onStartCommand(intent:Intent?,flags:Int,startId:Int):Int {
        if(intent?.action=="stop"){stopSelf();return START_NOT_STICKY}
        if(stopped||used||intent==null){stopSelf();return START_NOT_STICKY}
        used=true
        try {
            val code=intent.getIntExtra("resultCode",Activity.RESULT_CANCELED)
            val data=if(Build.VERSION.SDK_INT>=33)intent.getParcelableExtra("resultData",Intent::class.java)else{@Suppress("DEPRECATION") intent.getParcelableExtra("resultData") as Intent?}
            require(code==Activity.RESULT_OK&&data!=null){"Fresh MediaProjection consent required"}
            ticket=intent.getStringExtra("ticket")?:throw IllegalArgumentException("Capture request missing")
            thread=HandlerThread("LocalConnectCapture").apply{start()};handler=Handler(thread!!.looper)
            projection=getSystemService(MediaProjectionManager::class.java).getMediaProjection(code,data)
            projection!!.registerCallback(projectionCallback,main)
            display=NativeHooks.geometry(this)
            val size=scaled(display)
            reader=createReader(size.first,size.second,display)
            virtual=projection!!.createVirtualDisplay("LocalConnect approved LAN sharing",size.first,size.second,resources.configuration.densityDpi,DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,reader!!.surface,null,main)
            wake=getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"LocalConnect:projection").apply{setReferenceCounted(false);acquire(10*60*1000L)}
            main.postDelayed(renew,60_000)
            NativeHooks.projectionReady?.invoke(mapOf("ok" to true,"ticket" to ticket!!,"geometry" to display))
        }catch(e:Exception){NativeHooks.projectionReady?.invoke(mapOf("ok" to false,"error" to "Screen capture failed: ${e.message}"));stopSelf()}
        return START_NOT_STICKY
    }
    private fun scaled(geometry:Map<String,Any>):Pair<Int,Int>{
        val width=geometry["width"] as Int;val height=geometry["height"] as Int
        require(width>0&&height>0&&width<=16384&&height<=16384)
        val ratio=min(1.0,min(1920.0/width,1080.0/height))
        return Pair((width*ratio).toInt().coerceAtLeast(1),(height*ratio).toInt().coerceAtLeast(1))
    }
    private fun createReader(width:Int,height:Int,frameGeometry:Map<String,Any>):ImageReader {
        return ImageReader.newInstance(width,height,PixelFormat.RGBA_8888,2).apply{setOnImageAvailableListener({source->
            val image=try{source.acquireLatestImage()}catch(_:IllegalStateException){null}?:return@setOnImageAvailableListener
            try {
                if(stopped||source!==reader||frameGeometry!=display||SystemClock.elapsedRealtime()-last<250)return@setOnImageAvailableListener
                last=SystemClock.elapsedRealtime()
                val plane=image.planes[0];require(plane.pixelStride==4)
                val bytes=ByteArray(image.width*image.height*4)
                for(row in 0 until image.height){plane.buffer.position(row*plane.rowStride);plane.buffer.get(bytes,row*image.width*4,image.width*4)}
                val allocated=mutableListOf<Bitmap>()
                try {
                    val raw=Bitmap.createBitmap(image.width,image.height,Bitmap.Config.ARGB_8888).also{allocated.add(it)}
                    raw.copyPixelsFromBuffer(ByteBuffer.wrap(bytes))
                    val rotation=frameGeometry["rotation"] as Int
                    val canonical=if(rotation==0)raw else Bitmap.createBitmap(raw,0,0,raw.width,raw.height,Matrix().apply{postRotate(rotation.toFloat())},false).also{allocated.add(it)}
                    val ratio=min(1.0,min(1920.0/canonical.width,1080.0/canonical.height))
                    val bitmap=if(ratio<1)Bitmap.createScaledBitmap(canonical,(canonical.width*ratio).toInt().coerceAtLeast(1),(canonical.height*ratio).toInt().coerceAtLeast(1),true).also{allocated.add(it)}else canonical
                    val directory=File(filesDir,"screens").apply{mkdirs()}
                    val file=File(directory,"capture-$ticket-${sequence%2}.png");val temporary=File(directory,"capture-$ticket-${sequence%2}.tmp")
                    temporary.outputStream().use{check(bitmap.compress(Bitmap.CompressFormat.PNG,100,it))}
                    check(temporary.renameTo(file)){"Cannot publish capture frame"}
                    NativeHooks.frame?.invoke(mapOf("ticket" to (ticket?:""),"path" to file.absolutePath,"width" to bitmap.width,"height" to bitmap.height,"geometry" to frameGeometry,"sequence" to sequence++))
                } finally {allocated.distinct().forEach{it.recycle()}}
            }catch(e:Exception){main.post{NativeHooks.projectionReady?.invoke(mapOf("ok" to false,"error" to "Capture/storage error: ${e.message}"));stopSelf()}}
            finally{image.close()}
        },handler)}
    }
    private fun resize(){
        val geometry=NativeHooks.geometry(this)
        handler?.post {
            if(stopped||projection==null||virtual==null||geometry==display)return@post
            try {
                val size=scaled(geometry);val replacement=createReader(size.first,size.second,geometry)
                try {
                    virtual!!.resize(size.first,size.second,resources.configuration.densityDpi)
                    virtual!!.surface=replacement.surface
                    val old=reader;display=geometry;reader=replacement;old?.close()
                } catch(e:Exception) {replacement.close();throw e}
            }catch(e:Exception){main.post{NativeHooks.projectionReady?.invoke(mapOf("ok" to false,"error" to "Display resize failed: ${e.message}"));stopSelf()}}
        }
    }
    override fun onConfigurationChanged(config:Configuration){super.onConfigurationChanged(config);resize()}
    override fun onDestroy(){
        stopped=true;main.removeCallbacks(renew)
        val oldTicket=ticket;ticket=null
        val inputSession=RemoteAccessibilityService.approvedSession
        RemoteAccessibilityService.authorize(null)
        RustLifecycle.stopSession(inputSession)
        virtual?.release();virtual=null;reader?.close();reader=null
        val old=projection;projection=null;old?.unregisterCallback(projectionCallback);old?.stop()
        val oldThread=thread;thread=null
        handler?.post{if(oldTicket!=null){File(filesDir,"screens").listFiles()?.filter{it.name.startsWith("capture-$oldTicket-")}?.forEach{it.delete()}};oldThread?.quitSafely()};handler=null
        if(wake?.isHeld==true)wake?.release();wake=null
        stopForeground(STOP_FOREGROUND_REMOVE)
        if(oldTicket!=null)NativeHooks.projectionStopped?.invoke(oldTicket)
        super.onDestroy()
    }
    override fun onBind(intent:Intent?):IBinder?=null
}
