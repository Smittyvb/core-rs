use beserial::{Deserialize, Serialize};
use clear_on_drop::clear::Clear;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use nimiq_hash::argon2kdf::{compute_argon2_kdf, Argon2Error};
use rand::rngs::OsRng;
use rand::RngCore;

pub trait Verify {
    fn verify(&self) -> bool;
}

// Own ClearOnDrop
struct ClearOnDrop<T: Clear> {
    place: Option<T>
}

impl<T: Clear> ClearOnDrop<T> {
    #[inline]
    fn new(place: T) -> Self {
        ClearOnDrop {
            place: Some(place),
        }
    }

    #[inline]
    fn into_uncleared_place(mut c: Self) -> T {
        // By invariance, c.place must be Some(...).
        c.place.take().unwrap()
    }
}

impl<T: Clear> Drop for ClearOnDrop<T> {
    #[inline]
    fn drop(&mut self) {
        // Make sure to drop the unlocked data.
        if let Some(ref mut data) = self.place {
            data.clear();
        }
    }
}

impl<T: Clear> Deref for ClearOnDrop<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // By invariance, c.place must be Some(...).
        self.place.as_ref().unwrap()
    }
}

impl<T: Clear> DerefMut for ClearOnDrop<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // By invariance, c.place must be Some(...).
        self.place.as_mut().unwrap()
    }
}

// Unlocked container
pub struct Unlocked<T: Clear + Deserialize + Serialize> {
    data: ClearOnDrop<T>,
    lock: Locked<T>,
}

impl<T: Clear + Deserialize + Serialize> Unlocked<T> {
    #[inline]
    pub fn lock(lock: Self) -> Locked<T> {
        // ClearOnDrop makes sure the unlocked data is not leaked.
        lock.lock
    }

    #[inline]
    pub fn into_otp_lock(lock: Self) -> OtpLock<T> {
        OtpLock::Unlocked(lock)
    }

    #[inline]
    pub fn into_unlocked_data(lock: Self) -> T {
        ClearOnDrop::into_uncleared_place(lock.data)
    }
}

impl<T: Clear + Deserialize + Serialize> Deref for Unlocked<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

// Locked container
pub struct Locked<T: Clear + Deserialize + Serialize> {
    lock: Vec<u8>,
    salt: Vec<u8>,
    iterations: u32,
    phantom: PhantomData<T>,
}

impl<T: Clear + Deserialize + Serialize> Locked<T> {
    /// Calling code should make sure to clear the password from memory after use.
    /// The integrity of the output value is not checked.
    pub fn unlock_unchecked(self, password: &[u8]) -> Result<Unlocked<T>, Locked<T>> {
        let key_opt = Self::otp(&self.lock, password, self.iterations, &self.salt).ok();
        let mut key;
        if let Some(key_content) = key_opt {
            key = key_content;
        } else {
            return Err(self);
        }

        let result = Deserialize::deserialize_from_vec(&key).ok();

        // Always overwrite unencrypted vector.
        for i in 0..key.len() {
            key[i].clear();
        }

        if let Some(data) = result {
            Ok(Unlocked {
                data: ClearOnDrop::new(data),
                lock: self,
            })
        } else {
            Err(self)
        }
    }

    fn otp(secret: &[u8], password: &[u8], iterations: u32, salt: &[u8]) -> Result<Vec<u8>, Argon2Error> {
        let mut key = compute_argon2_kdf(password, salt, iterations, secret.len())?;
        assert_eq!(key.len(), secret.len());

        for (i, byte) in secret.iter().enumerate() {
            key[i] = key[i] ^ *byte;
        }

        Ok(key)
    }

    fn lock(secret: &T, password: &[u8], iterations: u32, salt: Vec<u8>) -> Result<Self, Argon2Error> {
        let mut data = secret.serialize_to_vec();
        let lock = Self::otp(&data, password, iterations, &salt)?;

        // Always overwrite unencrypted vector.
        for i in 0..data.len() {
            data[i].clear();
        }

        Ok(Locked {
            lock,
            salt,
            iterations,
            phantom: PhantomData,
        })
    }

    fn new(secret: &T, password: &[u8], iterations: u32, salt_length: usize) -> Result<Self, Argon2Error> {
        let mut csprng: OsRng = OsRng::new().unwrap();
        let mut salt = vec![0; salt_length];
        csprng.fill_bytes(salt.as_mut_slice());
        Self::lock(&secret, password, iterations, salt)
    }

    pub fn into_otp_lock(self) -> OtpLock<T> {
        OtpLock::Locked(self)
    }
}

impl<T: Clear + Deserialize + Serialize + Verify> Locked<T> {
    /// Verifies integrity of data upon unlock.
    pub fn unlock(self, password: &[u8]) -> Result<Unlocked<T>, Locked<T>> {
        let unlocked = self.unlock_unchecked(password);
        match unlocked {
            Ok(unlocked) => {
                if unlocked.verify() {
                    Ok(unlocked)
                } else {
                    Err(unlocked.lock)
                }
            },
            err => err,
        }
    }
}

// Generic container
pub enum OtpLock<T: Clear + Deserialize + Serialize> {
    Unlocked(Unlocked<T>),
    Locked(Locked<T>),
}

impl<T: Clear + Deserialize + Serialize> OtpLock<T> {
    // Taken from Nimiq's JS implementation.
    // TODO: Adjust.
    pub const DEFAULT_ITERATIONS: u32 = 256;
    pub const DEFAULT_SALT_LENGTH: usize = 32;

    pub fn new_unlocked(secret: T, password: &[u8], iterations: u32, salt_length: usize) -> Result<Self, Argon2Error> {
        let locked = Locked::new(&secret, password, iterations, salt_length)?;
        Ok(OtpLock::Unlocked(Unlocked {
            data: ClearOnDrop::new(secret),
            lock: locked,
        }))
    }

    pub fn new_unlocked_with_defaults(secret: T, password: &[u8]) -> Result<Self, Argon2Error> {
        Self::new_unlocked(secret, password, Self::DEFAULT_ITERATIONS, Self::DEFAULT_SALT_LENGTH)
    }

    /// Calling code should make sure to clear the password from memory after use.
    pub fn new_locked(mut secret: T, password: &[u8], iterations: u32, salt_length: usize) -> Result<Self, Argon2Error> {
        let result = Locked::new(&secret, password, iterations, salt_length)?;

        // Remove secret from memory.
        secret.clear();

        Ok(OtpLock::Locked(result))
    }

    /// Calling code should make sure to clear the password from memory after use.
    pub fn new_locked_with_defaults(secret: T, password: &[u8]) -> Result<Self, Argon2Error> {
        Self::new_locked(secret, password, Self::DEFAULT_ITERATIONS, Self::DEFAULT_SALT_LENGTH)
    }

    #[inline]
    pub fn is_locked(&self) -> bool {
        match self {
            OtpLock::Locked(_) => true,
            _ => false,
        }
    }

    #[inline]
    pub fn is_unlocked(&self) -> bool {
        !self.is_locked()
    }

    #[inline]
    pub fn lock(self) -> Self {
        match self {
            OtpLock::Unlocked(unlocked) => OtpLock::Locked(Unlocked::lock(unlocked)),
            l => l,
        }
    }

    #[inline]
    pub fn locked(self) -> Locked<T> {
        match self {
            OtpLock::Unlocked(unlocked) => Unlocked::lock(unlocked),
            OtpLock::Locked(locked) => locked,
        }
    }

    #[inline]
    pub fn unlocked(self) -> Result<Unlocked<T>, Self> {
        match self {
            OtpLock::Unlocked(unlocked) => Ok(unlocked),
            l => Err(l),
        }
    }

    #[inline]
    pub fn unlocked_ref(&self) -> Option<&Unlocked<T>> {
        match self {
            OtpLock::Unlocked(unlocked) => Some(&unlocked),
            _ => None,
        }
    }
}