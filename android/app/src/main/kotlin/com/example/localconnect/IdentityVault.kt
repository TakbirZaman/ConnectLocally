package com.example.localconnect

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.io.FileOutputStream
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** OS-backed wrapping; transient plaintext is removed immediately after Rust loads it. */
object IdentityVault {
    private const val ALIAS="LocalConnect identity wrapping v1"
    private fun key():SecretKey {
        val store=KeyStore.getInstance("AndroidKeyStore").apply{load(null)}
        if(store.containsAlias(ALIAS))return store.getKey(ALIAS,null) as SecretKey
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore").apply{
            init(KeyGenParameterSpec.Builder(ALIAS,KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    @Synchronized fun prepare(context:Context) {
        val dir=File(context.filesDir,"identity").apply{mkdirs()}
        val sealed=File(dir,"identity.key.sealed");val plain=File(dir,"identity.key")
        if(!sealed.exists())return // first initialization or migration from private plaintext.
        val bytes=sealed.readBytes();require(bytes.size in 29..4096&&bytes[0].toInt()==1)
        val cipher=Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE,key(),GCMParameterSpec(128,bytes.copyOfRange(1,13)))
        val decoded=cipher.doFinal(bytes.copyOfRange(13,bytes.size))
        FileOutputStream(plain).use{it.write(decoded);it.fd.sync()};decoded.fill(0)
        plain.setReadable(false,false);plain.setWritable(false,false);plain.setReadable(true,true);plain.setWritable(true,true)
    }
    @Synchronized fun seal(context:Context) {
        val dir=File(context.filesDir,"identity");val plain=File(dir,"identity.key")
        if(!plain.exists())return
        val bytes=plain.readBytes();require(bytes.size in 1..2048)
        val cipher=Cipher.getInstance("AES/GCM/NoPadding");cipher.init(Cipher.ENCRYPT_MODE,key())
        val encoded=byteArrayOf(1)+cipher.iv+cipher.doFinal(bytes);bytes.fill(0)
        val temp=File(dir,"identity.key.sealed.tmp");val sealed=File(dir,"identity.key.sealed")
        FileOutputStream(temp).use{it.write(encoded);it.fd.sync()}
        check(temp.renameTo(sealed)){"Cannot commit protected identity"}
        check(plain.delete()){ "Cannot remove transient identity key" }
    }
}
